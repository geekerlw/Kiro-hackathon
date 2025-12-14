use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{Mutex, mpsc, RwLock};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::types::{
    VideoSegment, AudioSegment, ProtocolMessage, MessageType, 
    QUICConnection, StreamType, KeyframeIndex, KeyframeEntry, FrameType, IndexOptimizationStrategy
};
use crate::errors::{TransportError, FileError, UploadManagerError, TimelineError};
use crate::file_reader::{FileStreamReader, DefaultFileStreamReader};
use crate::segmenter::{VideoSegmenter, DefaultVideoSegmenter};
use crate::separator::{AudioVideoSeparator, DefaultAudioVideoSeparator};
use crate::transport::{QUICTransport, DefaultQUICTransport};
use crate::controller::{PlaybackController, DefaultPlaybackController};
use crate::monitor::{PerformanceMonitor, DefaultPerformanceMonitor};
use crate::timeline_manager::{TimelineManager, CacheStats};
use crate::ffmpeg_cli_parser::TimelineData;
use crate::mock_server::{FileRequestPayload, PlaybackCommand, LiveStreamQuality};
use crate::live_encoder::{LiveH264Encoder, LiveEncoderConfig, OutputFormat};

/// 按需上传管理器 - 等待平台请求后才开始上传
pub struct OnDemandUploader {
    /// 文件读取器
    file_reader: Arc<DefaultFileStreamReader>,
    /// 视频分片器
    segmenter: Arc<DefaultVideoSegmenter>,
    /// 音视频分离器
    separator: Arc<DefaultAudioVideoSeparator>,
    /// QUIC传输层
    transport: Arc<DefaultQUICTransport>,
    /// 播放控制器
    controller: Arc<Mutex<DefaultPlaybackController>>,
    /// 性能监控器
    monitor: Arc<Mutex<DefaultPerformanceMonitor>>,
    /// 时间轴管理器
    timeline_manager: Arc<TimelineManager>,
    /// 活跃的上传会话
    active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
    /// 可用文件注册表
    file_registry: Arc<RwLock<HashMap<String, LocalFileInfo>>>,
    /// 控制消息接收器
    control_receiver: Option<mpsc::Receiver<PlatformMessage>>,
    /// 服务器连接
    server_connection: Option<QUICConnection>,
    /// 实时编码器
    live_encoder: Arc<Mutex<Option<LiveH264Encoder>>>,
    /// 活跃的直播会话
    live_sessions: Arc<RwLock<HashMap<String, LiveSession>>>,
}

/// 上传会话信息
#[derive(Debug, Clone)]
pub struct UploadSession {
    pub id: Uuid,
    pub file_path: String,
    pub status: UploadStatus,
    pub current_position: f64,
    pub playback_rate: f64,
    pub total_segments: u64,
    pub uploaded_segments: u64,
    pub start_time: std::time::SystemTime,
    pub video_stream_id: Option<u64>,
    pub audio_stream_id: Option<u64>,
    // 新增：准确的时间统计
    pub transmission_start_time: Option<std::time::Instant>,
    pub last_segment_time: Option<std::time::Instant>,
    pub total_bytes_transmitted: u64,
    pub average_segment_time_ms: f64,
    pub peak_throughput_mbps: f64,
    // 新增：关键帧索引和seek结果
    pub keyframe_index: Option<crate::types::KeyframeIndex>,
    pub last_seek_result: Option<crate::types::SeekResult>,
}

/// 上传状态
#[derive(Debug, Clone)]
pub enum UploadStatus {
    Preparing,
    Active,
    Paused,
    Seeking,
    RateChanging,
    Completed,
    Error(String),
}

/// 本地文件信息
#[derive(Debug, Clone)]
pub struct LocalFileInfo {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub duration: Option<f64>,
    pub format: String,
    pub available: bool,
    pub metadata: crate::types::VideoFileInfo,
    /// 时间轴数据 (如果已生成)
    pub timeline_data: Option<TimelineData>,
    /// 时间轴文件路径
    pub timeline_file_path: Option<PathBuf>,
}

/// 直播会话信息
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub stream_id: String,
    pub session_id: Uuid,
    pub quality: LiveStreamQuality,
    pub start_time: SystemTime,
    pub frames_transmitted: u64,
    pub bytes_transmitted: u64,
    pub is_active: bool,
    pub timestamp_overlay: bool,
}

/// 平台消息
#[derive(Debug, Clone)]
pub enum PlatformMessage {
    FileRequest {
        session_id: Uuid,
        file_path: String,
        priority: u8,
        seek_position: Option<f64>,
        playback_rate: f64,
    },
    PlaybackControl {
        session_id: Uuid,
        command: PlaybackCommand,
    },
    LiveStreamControl {
        session_id: Uuid,
        command: PlaybackCommand,
    },
    SessionEnd {
        session_id: Uuid,
    },
    FileListQuery {
        session_id: Uuid,
        filter: Option<String>,
    },
}

impl OnDemandUploader {
    /// 创建新的按需上传管理器
    pub fn new() -> Self {
        Self {
            file_reader: Arc::new(DefaultFileStreamReader::new()),
            segmenter: Arc::new(DefaultVideoSegmenter::new()),
            separator: Arc::new(DefaultAudioVideoSeparator::new()),
            transport: Arc::new(DefaultQUICTransport::new()),
            controller: Arc::new(Mutex::new(DefaultPlaybackController::new())),
            monitor: Arc::new(Mutex::new(DefaultPerformanceMonitor::new())),
            timeline_manager: Arc::new(TimelineManager::new()),
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            file_registry: Arc::new(RwLock::new(HashMap::new())),
            control_receiver: None,
            server_connection: None,
            live_encoder: Arc::new(Mutex::new(None)),
            live_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 连接到平台服务器
    pub async fn connect_to_platform(&mut self, server_addr: std::net::SocketAddr) -> Result<(), TransportError> {
        info!("Connecting to platform server at {}", server_addr);
        
        let connection = self.transport.connect(
            server_addr,
            crate::types::QUICOptions {
                max_concurrent_streams: 100,
                initial_max_data: 50 * 1024 * 1024, // 50MB - increased for large file transfers
                initial_max_stream_data: 5 * 1024 * 1024, // 5MB - increased to support 1MB+ segments
                idle_timeout: std::time::Duration::from_secs(30),
            }
        ).await?;

        self.server_connection = Some(connection);
        
        // 启动消息处理循环
        self.start_message_handling().await?;
        
        info!("Successfully connected to platform server");
        Ok(())
    }

    /// 注册本地可用文件
    pub async fn register_local_file(&self, file_path: PathBuf) -> Result<(), FileError> {
        info!("Registering local file: {:?}", file_path);
        
        // 读取文件信息
        let mut file_handle = self.file_reader.open_file(&file_path).await?;
        let metadata = self.file_reader.get_file_info(&mut file_handle).await?;
        self.file_reader.close_file(file_handle).await?;

        let format = file_path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("unknown")
            .to_lowercase();

        // 生成时间轴数据
        info!("Generating timeline for video file: {:?}", file_path);
        let timeline_data = match self.timeline_manager.get_timeline(&file_path).await {
            Ok(timeline) => {
                info!("Timeline generated successfully: {:.2}s duration, {} keyframes", 
                      timeline.total_duration, timeline.keyframes.len());
                Some(timeline)
            }
            Err(TimelineError::FFmpeg(crate::errors::FFmpegError::NotAvailable)) => {
                warn!("FFmpeg not available, timeline generation skipped for: {:?}", file_path);
                None
            }
            Err(e) => {
                warn!("Failed to generate timeline for {:?}: {}", file_path, e);
                None
            }
        };

        // 获取时间轴文件路径
        let timeline_file_path = if timeline_data.is_some() {
            let mut timeline_path = file_path.clone();
            timeline_path.set_extension("timeline");
            Some(timeline_path)
        } else {
            None
        };

        let has_timeline = timeline_data.is_some();
        
        let file_info = LocalFileInfo {
            file_size: std::fs::metadata(&file_path)
                .map_err(|e| FileError::IoError { message: e.to_string() })?
                .len(),
            duration: Some(metadata.duration),
            format,
            available: true,
            metadata,
            file_path: file_path.clone(),
            timeline_data,
            timeline_file_path,
        };

        let file_key = file_path.to_string_lossy().to_string();
        self.file_registry.write().await.insert(file_key.clone(), file_info);
        
        info!("Successfully registered file: {}", file_key);
        if has_timeline {
            info!("  - Timeline data: Available");
        } else {
            info!("  - Timeline data: Not available");
        }
        
        Ok(())
    }

    /// 获取可用文件列表
    pub async fn get_available_files(&self) -> Vec<String> {
        self.file_registry.read().await.keys().cloned().collect()
    }

    /// 获取文件的时间轴数据
    pub async fn get_file_timeline(&self, file_path: &str) -> Option<TimelineData> {
        let registry = self.file_registry.read().await;
        registry.get(file_path)?.timeline_data.clone()
    }

    /// 获取文件详细信息
    pub async fn get_file_info(&self, file_path: &str) -> Option<LocalFileInfo> {
        let registry = self.file_registry.read().await;
        registry.get(file_path).cloned()
    }

    /// 获取时间轴缓存统计信息
    pub async fn get_timeline_cache_stats(&self) -> CacheStats {
        self.timeline_manager.get_cache_stats().await
    }

    /// 预加载文件的时间轴 (后台任务)
    pub async fn preload_timeline(&self, file_path: &str) -> Result<(), TimelineError> {
        if let Some(file_info) = self.get_file_info(file_path).await {
            self.timeline_manager.preload_timeline(&file_info.file_path).await?;
        }
        Ok(())
    }

    /// 重新生成文件的时间轴
    pub async fn regenerate_timeline(&self, file_path: &str) -> Result<(), FileError> {
        let file_path_buf = PathBuf::from(file_path);
        
        // 清除缓存
        self.timeline_manager.invalidate_cache(&file_path_buf).await;
        
        // 重新生成
        let timeline_data = self.timeline_manager.get_timeline(&file_path_buf).await
            .map_err(|e| FileError::TimelineError(e))?;
        
        // 更新注册表
        let mut registry = self.file_registry.write().await;
        if let Some(file_info) = registry.get_mut(file_path) {
            file_info.timeline_data = Some(timeline_data);
            let mut timeline_path = file_path_buf.clone();
            timeline_path.set_extension("timeline");
            file_info.timeline_file_path = Some(timeline_path);
        }
        
        info!("Timeline regenerated for: {}", file_path);
        Ok(())
    }

    /// 启动消息处理
    async fn start_message_handling(&mut self) -> Result<(), TransportError> {
        let (tx, rx) = mpsc::channel(100);
        self.control_receiver = Some(rx);

        // 启动消息接收循环
        if let Some(connection) = &self.server_connection {
            let connection = connection.clone();
            let sender = tx.clone();
            
            tokio::spawn(async move {
                Self::handle_incoming_messages(connection, sender).await;
            });
        }

        // 启动消息处理循环
        let active_sessions = self.active_sessions.clone();
        let file_registry = self.file_registry.clone();
        let file_reader = self.file_reader.clone();
        let segmenter = self.segmenter.clone();
        let separator = self.separator.clone();
        let transport = self.transport.clone();
        let controller = self.controller.clone();
        let monitor = self.monitor.clone();

        if let Some(mut receiver) = self.control_receiver.take() {
            let connection = self.server_connection.clone().unwrap();
            tokio::spawn(async move {
                while let Some(message) = receiver.recv().await {
                    if let Err(e) = Self::process_platform_message(
                        message,
                        active_sessions.clone(),
                        file_registry.clone(),
                        file_reader.clone(),
                        segmenter.clone(),
                        separator.clone(),
                        transport.clone(),
                        controller.clone(),
                        monitor.clone(),
                        connection.clone(),
                    ).await {
                        error!("Error processing platform message: {}", e);
                    }
                }
            });
        }

        Ok(())
    }

    /// 处理传入消息
    async fn handle_incoming_messages(
        connection: QUICConnection,
        sender: mpsc::Sender<PlatformMessage>,
    ) {
        info!("Starting message handling for connection {}", connection.id);
        
        // 发送初始连接确认消息
        if let Err(e) = Self::send_connection_handshake(&connection).await {
            error!("Failed to send connection handshake: {}", e);
            return;
        }
        
        // 监听来自服务器的消息
        loop {
            match connection.inner.accept_uni().await {
                Ok(mut recv_stream) => {
                    match Self::receive_protocol_message(&mut recv_stream).await {
                        Ok(message) => {
                            if let Err(e) = Self::handle_protocol_message(message, &sender).await {
                                error!("Failed to handle protocol message: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to receive protocol message: {}", e);
                        }
                    }
                }
                Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                    info!("Server closed connection");
                    break;
                }
                Err(e) => {
                    error!("Connection error: {}", e);
                    break;
                }
            }
        }
    }
    
    /// 发送连接握手消息
    async fn send_connection_handshake(connection: &QUICConnection) -> Result<(), TransportError> {
        let handshake_msg = crate::types::ProtocolMessage {
            message_type: crate::types::MessageType::SessionStart,
            session_id: connection.id,
            timestamp: std::time::SystemTime::now(),
            sequence_number: 1,
            payload: b"CLIENT_READY".to_vec(),
        };
        
        let mut send_stream = connection.inner.open_uni().await
            .map_err(|e| TransportError::StreamCreationFailed { 
                reason: format!("Failed to open handshake stream: {}", e) 
            })?;
            
        let serialized = serde_json::to_vec(&handshake_msg)
            .map_err(|e| TransportError::SerializationError { 
                message: format!("Failed to serialize handshake: {}", e) 
            })?;
            
        send_stream.write_all(&serialized).await
            .map_err(|e| TransportError::NetworkError { 
                message: format!("Failed to send handshake: {}", e) 
            })?;
            
        send_stream.finish().await
            .map_err(|e| TransportError::NetworkError { 
                message: format!("Failed to finish handshake stream: {}", e) 
            })?;
            
        info!("Sent connection handshake for session {}", connection.id);
        Ok(())
    }
    
    /// 接收协议消息
    async fn receive_protocol_message(
        recv_stream: &mut quinn::RecvStream
    ) -> Result<crate::types::ProtocolMessage, TransportError> {
        let data = recv_stream.read_to_end(2 * 1024 * 1024).await // 2MB limit to accommodate large segments
            .map_err(|e| TransportError::NetworkError { 
                message: format!("Failed to read message: {}", e) 
            })?;
            
        serde_json::from_slice(&data)
            .map_err(|e| TransportError::SerializationError { 
                message: format!("Failed to deserialize message: {}", e) 
            })
    }
    
    /// 处理协议消息
    async fn handle_protocol_message(
        message: crate::types::ProtocolMessage,
        sender: &mpsc::Sender<PlatformMessage>,
    ) -> Result<(), TransportError> {
        use crate::types::MessageType;
        
        match message.message_type {
            MessageType::FileRequest => {
                let payload: crate::mock_server::FileRequestPayload = serde_json::from_slice(&message.payload)
                    .map_err(|e| TransportError::SerializationError { 
                        message: format!("Failed to parse file request: {}", e) 
                    })?;
                    
                let platform_msg = PlatformMessage::FileRequest {
                    session_id: message.session_id,
                    file_path: payload.file_path,
                    priority: payload.priority,
                    seek_position: payload.seek_position,
                    playback_rate: payload.playback_rate,
                };
                
                sender.send(platform_msg).await
                    .map_err(|e| TransportError::NetworkError { 
                        message: format!("Failed to forward file request: {}", e) 
                    })?;
            }
            
            MessageType::FileListQuery => {
                let payload: crate::mock_server::FileListQuery = serde_json::from_slice(&message.payload)
                    .map_err(|e| TransportError::SerializationError { 
                        message: format!("Failed to parse file list query: {}", e) 
                    })?;
                    
                let platform_msg = PlatformMessage::FileListQuery {
                    session_id: message.session_id,
                    filter: payload.filter,
                };
                
                sender.send(platform_msg).await
                    .map_err(|e| TransportError::NetworkError { 
                        message: format!("Failed to forward file list query: {}", e) 
                    })?;
            }
            
            MessageType::PlaybackControl => {
                let command: crate::mock_server::PlaybackCommand = serde_json::from_slice(&message.payload)
                    .map_err(|e| TransportError::SerializationError { 
                        message: format!("Failed to parse playback command: {}", e) 
                    })?;
                    
                let platform_msg = PlatformMessage::PlaybackControl {
                    session_id: message.session_id,
                    command,
                };
                
                sender.send(platform_msg).await
                    .map_err(|e| TransportError::NetworkError { 
                        message: format!("Failed to forward playback control: {}", e) 
                    })?;
            }
            
            MessageType::LiveStreamControl => {
                let command: crate::mock_server::PlaybackCommand = serde_json::from_slice(&message.payload)
                    .map_err(|e| TransportError::SerializationError { 
                        message: format!("Failed to parse live stream command: {}", e) 
                    })?;
                    
                let platform_msg = PlatformMessage::LiveStreamControl {
                    session_id: message.session_id,
                    command,
                };
                
                sender.send(platform_msg).await
                    .map_err(|e| TransportError::NetworkError { 
                        message: format!("Failed to forward live stream control: {}", e) 
                    })?;
            }
            
            MessageType::SessionEnd => {
                let platform_msg = PlatformMessage::SessionEnd {
                    session_id: message.session_id,
                };
                
                sender.send(platform_msg).await
                    .map_err(|e| TransportError::NetworkError { 
                        message: format!("Failed to forward session end: {}", e) 
                    })?;
            }
            
            _ => {
                warn!("Received unhandled message type: {:?}", message.message_type);
            }
        }
        
        Ok(())
    }
    
    /// 发送文件列表响应
    async fn send_file_list_response(
        connection: &QUICConnection,
        session_id: Uuid,
        files: Vec<crate::mock_server::ClientFileInfo>,
    ) -> Result<(), TransportError> {
        let response = crate::mock_server::FileListResponse { files };
        
        let response_msg = crate::types::ProtocolMessage {
            message_type: crate::types::MessageType::FileListResponse,
            session_id,
            timestamp: std::time::SystemTime::now(),
            sequence_number: 1,
            payload: serde_json::to_vec(&response)
                .map_err(|e| TransportError::SerializationError { 
                    message: format!("Failed to serialize file list response: {}", e) 
                })?,
        };
        
        let mut send_stream = connection.inner.open_uni().await
            .map_err(|e| TransportError::StreamCreationFailed { 
                reason: format!("Failed to open response stream: {}", e) 
            })?;
            
        let serialized = serde_json::to_vec(&response_msg)
            .map_err(|e| TransportError::SerializationError { 
                message: format!("Failed to serialize response message: {}", e) 
            })?;
            
        send_stream.write_all(&serialized).await
            .map_err(|e| TransportError::NetworkError { 
                message: format!("Failed to send response: {}", e) 
            })?;
            
        send_stream.finish().await
            .map_err(|e| TransportError::NetworkError { 
                message: format!("Failed to finish response stream: {}", e) 
            })?;
            
        info!("Sent file list response with {} files", response.files.len());
        Ok(())
    }

    // Continue with the existing loop structure
    async fn _message_loop() {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            
            // 在实际实现中，这里会从QUIC流中读取消息
            // 然后解析为PlatformMessage并发送给处理器
        }
    }

    /// 处理平台消息
    async fn process_platform_message(
        message: PlatformMessage,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        file_registry: Arc<RwLock<HashMap<String, LocalFileInfo>>>,
        file_reader: Arc<DefaultFileStreamReader>,
        segmenter: Arc<DefaultVideoSegmenter>,
        separator: Arc<DefaultAudioVideoSeparator>,
        transport: Arc<DefaultQUICTransport>,
        controller: Arc<Mutex<DefaultPlaybackController>>,
        monitor: Arc<Mutex<DefaultPerformanceMonitor>>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        match message {
            PlatformMessage::FileRequest { 
                session_id, 
                file_path, 
                priority: _,
                seek_position,
                playback_rate 
            } => {
                info!("Received file request for: {} (session: {})", file_path, session_id);
                
                // 检查文件是否可用
                let file_registry_guard = file_registry.read().await;
                if let Some(file_info) = file_registry_guard.get(&file_path) {
                    if !file_info.available {
                        warn!("Requested file is not available: {}", file_path);
                        return Ok(());
                    }
                    
                    // 创建上传会话
                    let session = UploadSession {
                        id: session_id,
                        file_path: file_path.clone(),
                        status: UploadStatus::Preparing,
                        current_position: seek_position.unwrap_or(0.0),
                        playback_rate,
                        total_segments: 0,
                        uploaded_segments: 0,
                        start_time: std::time::SystemTime::now(),
                        video_stream_id: None,
                        audio_stream_id: None,
                        // 初始化时间统计
                        transmission_start_time: None,
                        last_segment_time: None,
                        total_bytes_transmitted: 0,
                        average_segment_time_ms: 0.0,
                        peak_throughput_mbps: 0.0,
                        // 初始化关键帧索引和seek结果
                        keyframe_index: None,
                        last_seek_result: None,
                    };
                    
                    active_sessions.write().await.insert(session_id, session);
                    drop(file_registry_guard);
                    
                    // 启动上传任务
                    Self::start_upload_task(
                        session_id,
                        file_path,
                        seek_position,
                        playback_rate,
                        active_sessions,
                        file_registry,
                        file_reader,
                        segmenter,
                        separator,
                        transport,
                        controller,
                        monitor,
                        connection.clone(),
                    ).await?;
                } else {
                    warn!("Requested file not found in registry: {}", file_path);
                }
            }
            
            PlatformMessage::PlaybackControl { session_id, command } => {
                info!("Received playback control for session {}: {:?}", session_id, command);
                
                let mut sessions = active_sessions.write().await;
                if let Some(session) = sessions.get_mut(&session_id) {
                    match command {
                        PlaybackCommand::Seek { position } => {
                            session.status = UploadStatus::Seeking;
                            
                            let mut controller_guard = controller.lock().await;
                            
                            // 尝试使用精确seek功能（如果有关键帧索引）
                            let seek_result = if let Some(ref keyframe_index) = session.keyframe_index {
                                // 使用关键帧索引进行精确seek
                                match controller_guard.seek_to_keyframe(position, keyframe_index).await {
                                    Ok(result) => {
                                        info!("Precise seek completed: requested={:.3}s, actual={:.3}s, precision={:.3}", 
                                              result.requested_time, result.actual_time, result.precision_achieved);
                                        let actual_time = result.actual_time;
                                        session.current_position = actual_time;
                                        session.last_seek_result = Some(result);
                                        Ok(actual_time)
                                    }
                                    Err(e) => Err(e)
                                }
                            } else {
                                // 回退到基本seek功能
                                match controller_guard.seek(position).await {
                                    Ok(_) => {
                                        session.current_position = position;
                                        Ok(position)
                                    }
                                    Err(e) => Err(e)
                                }
                            };
                            
                            drop(controller_guard); // 释放controller锁
                            
                            match seek_result {
                                Ok(actual_position) => {
                                    session.status = UploadStatus::Active;
                                    
                                    // 重要：从新位置重新启动上传任务
                                    info!("Restarting upload from new position: {:.3}s for session {}", actual_position, session_id);
                                    
                                    // 启动新的上传任务从seek位置开始
                                    let file_path = session.file_path.clone();
                                    let playback_rate = session.playback_rate;
                                    
                                    Self::start_upload_task(
                                        session_id,
                                        file_path,
                                        Some(actual_position), // 从seek位置开始
                                        playback_rate,
                                        active_sessions.clone(),
                                        file_registry.clone(),
                                        file_reader.clone(),
                                        segmenter.clone(),
                                        separator.clone(),
                                        transport.clone(),
                                        controller.clone(),
                                        monitor.clone(),
                                        connection.clone(),
                                    ).await?;
                                }
                                Err(e) => {
                                    error!("Seek error: {}", e);
                                    session.status = UploadStatus::Error(e.to_string());
                                }
                            }
                        }
                        
                        PlaybackCommand::SetRate { rate } => {
                            session.status = UploadStatus::RateChanging;
                            session.playback_rate = rate;
                            
                            let mut controller_guard = controller.lock().await;
                            if let Err(e) = controller_guard.set_playback_rate(rate).await {
                                error!("Rate change error: {}", e);
                                session.status = UploadStatus::Error(e.to_string());
                            } else {
                                session.status = UploadStatus::Active;
                            }
                        }
                        
                        PlaybackCommand::Pause => {
                            session.status = UploadStatus::Paused;
                        }
                        
                        PlaybackCommand::Resume => {
                            session.status = UploadStatus::Active;
                        }
                        
                        PlaybackCommand::Stop => {
                            session.status = UploadStatus::Completed;
                        }
                        
                        PlaybackCommand::StartLive { .. } | PlaybackCommand::StopLive { .. } => {
                            // 这些命令应该通过 LiveStreamControl 消息处理，而不是 PlaybackControl
                            warn!("Received live stream command via PlaybackControl, should use LiveStreamControl");
                        }
                    }
                }
            }
            
            PlatformMessage::LiveStreamControl { session_id, command } => {
                info!("Received live stream control for session {}: {:?}", session_id, command);
                
                match command {
                    PlaybackCommand::StartLive { stream_id, quality, timestamp_overlay } => {
                        info!("Starting live stream: {} for session {}", stream_id, session_id);
                        
                        // 启动直播编码任务
                        Self::start_live_stream(
                            session_id,
                            stream_id,
                            quality,
                            timestamp_overlay,
                            active_sessions.clone(),
                            transport.clone(),
                            connection.clone(),
                        ).await?;
                    }
                    
                    PlaybackCommand::StopLive { stream_id } => {
                        info!("Stopping live stream: {} for session {}", stream_id, session_id);
                        
                        // 停止直播编码任务
                        Self::stop_live_stream(
                            session_id,
                            stream_id,
                            active_sessions.clone(),
                        ).await?;
                    }
                    
                    _ => {
                        warn!("Unsupported live stream command: {:?}", command);
                    }
                }
            }
            
            PlatformMessage::SessionEnd { session_id } => {
                info!("Ending session: {}", session_id);
                active_sessions.write().await.remove(&session_id);
            }
            
            PlatformMessage::FileListQuery { session_id, filter } => {
                info!("Received file list query from session: {}", session_id);
                
                // 收集可用文件信息
                let file_registry_guard = file_registry.read().await;
                let mut available_files = Vec::new();
                
                for (file_path, file_info) in file_registry_guard.iter() {
                    // 应用过滤器（如果有）
                    let matches_filter = if let Some(ref filter_str) = filter {
                        file_path.contains(filter_str) || 
                        file_info.format.contains(filter_str)
                    } else {
                        true
                    };
                    
                    if file_info.available && matches_filter {
                        available_files.push(crate::mock_server::ClientFileInfo {
                            file_path: file_path.clone(),
                            file_size: file_info.file_size,
                            duration: file_info.duration,
                            format: file_info.format.clone(),
                            available: file_info.available,
                        });
                    }
                }
                
                drop(file_registry_guard);
                
                info!("Sending file list response with {} files", available_files.len());
                for file in &available_files {
                    info!("  - {} ({} bytes, format: {})", 
                          file.file_path, file.file_size, file.format);
                }
                
                // 发送文件列表响应回服务器
                if let Err(e) = Self::send_file_list_response(&connection, session_id, available_files).await {
                    error!("Failed to send file list response: {}", e);
                } else {
                    info!("Successfully sent file list response to server");
                }
            }
        }
        
        Ok(())
    }

    /// 启动上传任务
    async fn start_upload_task(
        session_id: Uuid,
        file_path: String,
        seek_position: Option<f64>,
        playback_rate: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        file_registry: Arc<RwLock<HashMap<String, LocalFileInfo>>>,
        file_reader: Arc<DefaultFileStreamReader>,
        segmenter: Arc<DefaultVideoSegmenter>,
        separator: Arc<DefaultAudioVideoSeparator>,
        transport: Arc<DefaultQUICTransport>,
        controller: Arc<Mutex<DefaultPlaybackController>>,
        monitor: Arc<Mutex<DefaultPerformanceMonitor>>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        tokio::spawn(async move {
            if let Err(e) = Self::execute_upload(
                session_id,
                file_path,
                seek_position,
                playback_rate,
                active_sessions,
                file_registry,
                file_reader,
                segmenter,
                separator,
                transport,
                controller,
                monitor,
                connection,
            ).await {
                error!("Upload task failed for session {}: {}", session_id, e);
            }
        });
        
        Ok(())
    }

    /// 执行实际的上传过程
    async fn execute_upload(
        session_id: Uuid,
        file_path: String,
        seek_position: Option<f64>,
        playback_rate: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        file_registry: Arc<RwLock<HashMap<String, LocalFileInfo>>>,
        file_reader: Arc<DefaultFileStreamReader>,
        _segmenter: Arc<DefaultVideoSegmenter>,
        _separator: Arc<DefaultAudioVideoSeparator>,
        transport: Arc<DefaultQUICTransport>,
        controller: Arc<Mutex<DefaultPlaybackController>>,
        monitor: Arc<Mutex<DefaultPerformanceMonitor>>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting upload execution for session: {}", session_id);
        
        // 更新会话状态
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.status = UploadStatus::Active;
            }
        }

        // 获取文件信息
        let file_info = {
            let registry = file_registry.read().await;
            registry.get(&file_path).cloned()
                .ok_or_else(|| UploadManagerError::FileNotFound(file_path.clone()))?
        };

        // 打开文件
        let mut file_handle = file_reader.open_file(&file_info.file_path).await
            .map_err(|e| UploadManagerError::FileError(e))?;

        // 构建关键帧索引以支持精确seek
        info!("Building keyframe index for precise seek operations...");
        let keyframe_index = if let Some(ref timeline) = file_info.timeline_data {
            // 如果有timeline数据，使用真实的关键帧信息构建索引
            info!("Using timeline data to build precise keyframe index");
            Some(Self::build_keyframe_index_from_timeline(timeline))
        } else {
            // 回退到文件扫描方式
            match file_reader.build_keyframe_index(&mut file_handle).await {
                Ok(index) => {
                    info!("Keyframe index built successfully: {} keyframes, precision: {:.3}s", 
                          index.entries.len(), index.index_precision);
                    Some(index)
                }
                Err(e) => {
                    warn!("Failed to build keyframe index: {}. Falling back to basic seek.", e);
                    None
                }
            }
        };

        // 将关键帧索引存储到会话中
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.keyframe_index = keyframe_index.clone();
            }
        }

        // 如果有seek位置，使用精确seek功能
        if let Some(position) = seek_position {
            let mut controller_guard = controller.lock().await;
            
            if let Some(ref index) = keyframe_index {
                // 使用关键帧索引进行精确seek
                match controller_guard.seek_to_keyframe(position, index).await {
                    Ok(result) => {
                        info!("Initial precise seek completed: requested={:.3}s, actual={:.3}s, precision={:.3}", 
                              result.requested_time, result.actual_time, result.precision_achieved);
                        
                        // 更新会话中的实际位置和seek结果
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.current_position = result.actual_time;
                            session.last_seek_result = Some(result);
                        }
                    }
                    Err(e) => {
                        error!("Precise seek failed: {}", e);
                        return Err(UploadManagerError::PlaybackError(e));
                    }
                }
            } else {
                // 回退到基本seek
                controller_guard.seek(position).await
                    .map_err(|e| UploadManagerError::PlaybackError(e))?;
            }
        }

        // 设置播放速率
        if playback_rate != 1.0 {
            let mut controller_guard = controller.lock().await;
            controller_guard.set_playback_rate(playback_rate).await
                .map_err(|e| UploadManagerError::PlaybackError(e))?;
        }

        // 开始监控
        monitor.lock().await.start_monitoring();

        // 实际的文件传输和分片过程
        info!("Starting real file segmentation and transmission for: {}", file_path);
        
        // 真实文件传输过程
        Self::real_file_transmission(
            session_id,
            &file_info,
            file_handle,
            seek_position,
            playback_rate,
            active_sessions.clone(),
            file_reader,
            transport,
            connection,
        ).await?;

        info!("Upload execution completed for session: {}", session_id);
        
        // 更新会话状态为完成
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.status = UploadStatus::Completed;
            }
        }

        Ok(())
    }

    /// 获取活跃会话统计
    pub async fn get_active_sessions(&self) -> Vec<UploadSession> {
        self.active_sessions.read().await.values().cloned().collect()
    }

    /// 获取会话详细信息
    pub async fn get_session_info(&self, session_id: Uuid) -> Option<UploadSession> {
        self.active_sessions.read().await.get(&session_id).cloned()
    }

    /// 从timeline数据构建关键帧索引
    fn build_keyframe_index_from_timeline(timeline: &TimelineData) -> KeyframeIndex {
        
        let mut entries = Vec::new();
        
        // 将timeline中的关键帧转换为KeyframeEntry
        for keyframe in &timeline.keyframes {
            let entry = KeyframeEntry {
                timestamp: keyframe.timestamp,
                file_offset: keyframe.file_offset,
                frame_size: keyframe.frame_size,
                gop_size: 1, // 关键帧的GOP大小为1
                frame_type: FrameType::I, // 关键帧都是I帧
            };
            entries.push(entry);
        }
        
        // 计算索引精度（相邻关键帧的平均间隔）
        let index_precision = if entries.len() > 1 {
            timeline.total_duration / (entries.len() - 1) as f64
        } else {
            timeline.total_duration
        };
        
        // 计算内存使用量
        let memory_usage = std::mem::size_of::<KeyframeIndex>() + 
                          entries.len() * std::mem::size_of::<KeyframeEntry>();
        
        info!("Built keyframe index from timeline: {} keyframes, precision: {:.3}s", 
              entries.len(), index_precision);
        
        KeyframeIndex {
            entries,
            total_duration: timeline.total_duration,
            index_precision,
            memory_optimized: true,
            optimization_strategy: IndexOptimizationStrategy::Full,
            memory_usage,
        }
    }

    /// 模拟文件分片过程 - 优化为极低延迟
    async fn simulate_file_segmentation(
        file_info: &LocalFileInfo,
    ) -> Result<u64, UploadManagerError> {
        // 为极低延迟优化分片策略
        // 1. 基于时间的分片：每个分片包含很短的时间段
        let segment_duration_ms = 33.33; // ~30fps，每帧一个分片，约33ms
        let total_duration_ms = file_info.duration.unwrap_or(60.0) * 1000.0;
        let time_based_segments = (total_duration_ms / segment_duration_ms).ceil() as u64;
        
        // 2. 基于数据大小的分片：小块传输以减少延迟
        let low_latency_segment_size = 8 * 1024; // 8KB per segment for low latency
        let size_based_segments = (file_info.file_size + low_latency_segment_size - 1) / low_latency_segment_size;
        
        // 使用更细粒度的分片数量（取较大值以确保足够细致）
        let total_segments = std::cmp::max(time_based_segments, size_based_segments);
        
        info!("File {} optimized for low latency:", file_info.file_path.display());
        info!("  Duration: {:.2}s", file_info.duration.unwrap_or(0.0));
        info!("  File size: {} bytes", file_info.file_size);
        info!("  Time-based segments (33ms each): {}", time_based_segments);
        info!("  Size-based segments (8KB each): {}", size_based_segments);
        info!("  Total segments for transmission: {}", total_segments);
        info!("  Estimated latency per segment: ~33ms");
        
        // 减少分片处理时间以提高响应速度
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        
        Ok(total_segments)
    }

    /// 模拟分片传输过程
    async fn simulate_segment_transmission(
        session_id: Uuid,
        total_segments: u64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting transmission of {} segments for session {}", total_segments, session_id);
        
        for segment_num in 1..=total_segments {
            // 检查会话状态，如果暂停则等待
            loop {
                let sessions = active_sessions.read().await;
                if let Some(session) = sessions.get(&session_id) {
                    match session.status {
                        UploadStatus::Active => break,
                        UploadStatus::Paused => {
                            drop(sessions);
                            info!("Session {} paused, waiting...", session_id);
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            continue;
                        }
                        UploadStatus::Completed => {
                            info!("Session {} completed, stopping transmission", session_id);
                            return Ok(());
                        }
                        _ => break,
                    }
                } else {
                    warn!("Session {} not found, stopping transmission", session_id);
                    return Ok(());
                }
            }

            // 创建极低延迟的分片数据
            let is_audio = segment_num % 4 == 0; // 每4个分片中有1个音频分片（更符合实际比例）
            let segment = if is_audio {
                crate::types::Segment::Audio(crate::types::AudioSegment {
                    id: Uuid::new_v4(),
                    data: vec![0u8; 512], // 512B 音频数据，减少延迟
                    timestamp: segment_num as f64 * 0.033, // 每个分片33ms（30fps）
                    duration: 0.033, // 33ms duration for low latency
                    sample_rate: 44100,
                    channels: 2,
                })
            } else {
                // 视频分片：每个分片对应一帧或几帧
                let frames_per_segment = 1; // 每个分片1帧，最低延迟
                crate::types::Segment::Video(crate::types::VideoSegment {
                    id: Uuid::new_v4(),
                    data: vec![0u8; 8192], // 8KB 视频数据，适合低延迟传输
                    timestamp: segment_num as f64 * 0.033, // 33ms per frame at 30fps
                    duration: 0.033, // 33ms duration
                    frame_count: frames_per_segment,
                    is_key_frame: segment_num % 30 == 1, // 每秒一个关键帧（30帧）
                    metadata: crate::types::SegmentMetadata {
                        frame_indices: vec![0], // 单帧分片
                        key_frame_positions: if segment_num % 30 == 1 { vec![0] } else { vec![] },
                        encoding_params: {
                            let mut params = std::collections::HashMap::new();
                            params.insert("low_latency".to_string(), "true".to_string());
                            params.insert("frame_rate".to_string(), "30".to_string());
                            params.insert("segment_duration_ms".to_string(), "33".to_string());
                            params
                        },
                    },
                })
            };

            // 实际发送分片数据到服务器
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    // 极低延迟模式：最大速度传输，仅在必要时让出CPU
                    if segment_num % 20 == 0 {
                        tokio::task::yield_now().await;
                    }

                    // 更新传输进度
                    {
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.uploaded_segments = segment_num;
                            
                            if is_audio {
                                info!("Transmitted audio frame {}/{} (33ms, 512B) for session {}", 
                                      segment_num, total_segments, session_id);
                            } else {
                                let is_keyframe = segment_num % 30 == 1;
                                info!("Transmitted video frame {}/{} (33ms, 8KB, {}) for session {}", 
                                      segment_num, total_segments, 
                                      if is_keyframe { "keyframe" } else { "frame" },
                                      session_id);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to send segment {}: {}", segment_num, e);
                    // 更新会话状态为错误
                    let mut sessions = active_sessions.write().await;
                    if let Some(session) = sessions.get_mut(&session_id) {
                        session.status = UploadStatus::Error(format!("Transmission failed: {}", e));
                    }
                    return Err(UploadManagerError::TransportError(e));
                }
            }
        }

        info!("All {} segments transmitted for session {}", total_segments, session_id);
        Ok(())
    }

    /// 真实文件传输过程 - 使用帧级分片和音视频分离
    async fn real_file_transmission(
        session_id: Uuid,
        file_info: &LocalFileInfo,
        _file_handle: tokio::fs::File,
        seek_position: Option<f64>,
        _playback_rate: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        _file_reader: Arc<DefaultFileStreamReader>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        let start_position = seek_position.unwrap_or(0.0);
        
        // 获取关键帧的file_offset（如果有seek位置且不为0）
        let start_file_offset = if let Some(position) = seek_position {
            // 只有当seek位置大于0时才使用offset，否则从文件开头读取
            if position > 0.0 {
                // 从会话中获取最后的seek结果
                let sessions = active_sessions.read().await;
                if let Some(session) = sessions.get(&session_id) {
                    if let Some(ref seek_result) = session.last_seek_result {
                        info!("Using keyframe file offset: {} for position {:.3}s", 
                              seek_result.keyframe_offset, position);
                        Some(seek_result.keyframe_offset)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                // seek位置为0，从文件开头读取
                info!("Seek position is 0, reading from file beginning");
                None
            }
        } else {
            // 没有seek位置，从文件开头读取
            info!("No seek position specified, reading from file beginning");
            None
        };
        
        info!("Starting intelligent file transmission for session {} with format: {} from position: {:.3}s", 
              session_id, file_info.format, start_position);
        
        // 读取文件内容（从指定偏移开始，如果有的话）
        let file_data = if let Some(offset) = start_file_offset {
            info!("Reading file from offset: {} bytes", offset);
            let mut file = std::fs::File::open(&file_info.file_path)
                .map_err(|e| UploadManagerError::FileError(
                    crate::errors::FileError::IoError { message: e.to_string() }
                ))?;
            
            use std::io::{Read, Seek, SeekFrom};
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| UploadManagerError::FileError(
                    crate::errors::FileError::IoError { message: e.to_string() }
                ))?;
            
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .map_err(|e| UploadManagerError::FileError(
                    crate::errors::FileError::IoError { message: e.to_string() }
                ))?;
            buffer
        } else {
            // 读取整个文件
            std::fs::read(&file_info.file_path)
                .map_err(|e| UploadManagerError::FileError(
                    crate::errors::FileError::IoError { message: e.to_string() }
                ))?
        };
        
        info!("Read {} bytes from file: {:?} (offset: {:?})", 
              file_data.len(), file_info.file_path, start_file_offset);
        
        // 统一使用固定大小分片策略，确保完整传输所有数据（包括SPS/PPS等）
        match file_info.format.as_str() {
            "h264" => {
                info!("Processing H.264 file with fixed-size segmentation (ensures SPS/PPS transmission)");
                // H.264也使用固定分片策略，确保SPS/PPS等参数集被完整传输
                Self::process_mp4_file_from_position(
                    session_id,
                    &file_data,
                    start_position,
                    active_sessions,
                    transport,
                    connection,
                ).await
            }
            "mp4" => {
                info!("Processing MP4 file with fixed-size segmentation (high throughput mode)");
                Self::process_mp4_file_from_position(
                    session_id,
                    &file_data,
                    start_position,
                    active_sessions,
                    transport,
                    connection,
                ).await
            }
            _ => {
                warn!("Unknown format {}, falling back to generic segmentation", file_info.format);
                Self::process_generic_file_from_position(
                    session_id,
                    &file_data,
                    start_position,
                    active_sessions,
                    transport,
                    connection,
                ).await
            }
        }
    }
    
    /// 处理H.264文件 - 真正的帧级分片
    async fn process_h264_file(
        session_id: Uuid,
        file_data: &[u8],
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        Self::process_h264_file_from_position(session_id, file_data, 0.0, active_sessions, transport, connection).await
    }

    /// 处理H.264文件 - 从指定位置开始的帧级分片
    async fn process_h264_file_from_position(
        session_id: Uuid,
        file_data: &[u8],
        start_position: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting H.264 frame-level processing for session {} from position {:.3}s", session_id, start_position);
        
        // 创建视频分片器
        let segmenter = DefaultVideoSegmenter::with_frame_rate(30.0);
        
        // 解析H.264帧
        let frames = segmenter.parse_h264_frames(file_data);
        info!("Found {} H.264 frames in file", frames.len());
        
        // 计算起始帧索引（基于30fps）
        let start_frame_index = (start_position * 30.0) as usize;
        let frames_to_transmit = if start_frame_index < frames.len() {
            &frames[start_frame_index..]
        } else {
            info!("Start position {:.3}s is beyond file duration, starting from beginning", start_position);
            &frames[..]
        };
        
        info!("Starting transmission from frame {} (position {:.3}s), {} frames remaining", 
              start_frame_index, start_position, frames_to_transmit.len());
        
        // 记录传输开始时间并更新会话信息
        let transmission_start = std::time::Instant::now();
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.total_segments = frames_to_transmit.len() as u64;
                session.transmission_start_time = Some(transmission_start);
                session.total_bytes_transmitted = 0;
            }
        }
        
        // 逐帧传输（从指定位置开始）
        for (relative_index, (frame_pos, is_key_frame)) in frames_to_transmit.iter().enumerate() {
            let actual_frame_index = start_frame_index + relative_index;
            // 检查会话状态
            if !Self::check_session_active(session_id, &active_sessions).await? {
                return Ok(());
            }
            
            // 计算帧数据范围
            let frame_end = if actual_frame_index + 1 < frames.len() {
                frames[actual_frame_index + 1].0
            } else {
                file_data.len()
            };
            
            let frame_data = &file_data[*frame_pos..frame_end];
            
            // 提取编码参数
            let encoding_params = segmenter.extract_encoding_params(frame_data);
            
            // 创建帧级视频分片（使用实际时间戳）
            let segment = crate::types::Segment::Video(crate::types::VideoSegment {
                id: Uuid::new_v4(),
                data: frame_data.to_vec(),
                timestamp: actual_frame_index as f64 / 30.0, // 使用实际帧索引计算时间戳
                duration: 1.0 / 30.0, // 33.33ms per frame
                frame_count: 1,
                is_key_frame: *is_key_frame,
                metadata: crate::types::SegmentMetadata {
                    frame_indices: vec![actual_frame_index],
                    key_frame_positions: if *is_key_frame { vec![0] } else { vec![] },
                    encoding_params,
                },
            });
            
            // 发送帧到服务器
            let segment_start = std::time::Instant::now();
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    let segment_end = std::time::Instant::now();
                    let segment_duration = segment_end.duration_since(segment_start);
                    
                    // 更新传输进度和统计信息
                    {
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.uploaded_segments = (relative_index + 1) as u64;
                            session.last_segment_time = Some(segment_end);
                            session.total_bytes_transmitted += frame_data.len() as u64;
                            
                            // 计算平均分片传输时间
                            let total_time = if let Some(start) = session.transmission_start_time {
                                segment_end.duration_since(start).as_millis() as f64
                            } else {
                                segment_duration.as_millis() as f64
                            };
                            session.average_segment_time_ms = total_time / session.uploaded_segments as f64;
                            
                            // 计算当前吞吐量 (Mbps)
                            let segment_throughput = if segment_duration.as_millis() > 0 {
                                (frame_data.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                            } else {
                                0.0
                            };
                            
                            if segment_throughput > session.peak_throughput_mbps {
                                session.peak_throughput_mbps = segment_throughput;
                            }
                        }
                    }
                    
                    info!("Transmitted H.264 frame {}/{} (actual frame {}, {:.3}s, {} bytes, {}, {:.2}ms, {:.1}Mbps) for session {}", 
                          relative_index + 1, frames_to_transmit.len(), actual_frame_index + 1, 
                          actual_frame_index as f64 / 30.0, frame_data.len(),
                          if *is_key_frame { "KEY" } else { "P/B" },
                          segment_duration.as_millis(),
                          (frame_data.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0,
                          session_id);
                    
                    // 极低延迟模式：最大速度传输，仅在必要时让出CPU
                    if relative_index % 10 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                Err(e) => {
                    error!("Failed to send H.264 frame {} (actual frame {}): {}", relative_index + 1, actual_frame_index + 1, e);
                    Self::mark_session_error(session_id, &active_sessions, e.to_string()).await;
                    return Err(UploadManagerError::TransportError(e));
                }
            }
        }
        
        // 计算并显示传输统计
        let (total_time, total_bytes, avg_time, peak_throughput) = {
            let sessions = active_sessions.read().await;
            if let Some(session) = sessions.get(&session_id) {
                let total_time = if let Some(start) = session.transmission_start_time {
                    std::time::Instant::now().duration_since(start)
                } else {
                    std::time::Duration::from_millis(0)
                };
                (total_time, session.total_bytes_transmitted, session.average_segment_time_ms, session.peak_throughput_mbps)
            } else {
                (std::time::Duration::from_millis(0), 0, 0.0, 0.0)
            }
        };
        
        let overall_throughput = if total_time.as_millis() > 0 {
            (total_bytes as f64 * 8.0) / (total_time.as_millis() as f64 / 1000.0) / 1_000_000.0
        } else {
            0.0
        };
        
        info!("H.264 frame-level transmission completed for session {} ({} frames, {} bytes, {:.2}s, avg {:.2}ms/frame, {:.1}Mbps overall, {:.1}Mbps peak)", 
              session_id, frames.len(), total_bytes, total_time.as_secs_f64(), avg_time, overall_throughput, peak_throughput);
        Ok(())
    }
    
    /// 处理MP4文件 - 使用固定大小分片（高效传输）
    async fn process_mp4_file(
        session_id: Uuid,
        file_data: &[u8],
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        Self::process_mp4_file_from_position(session_id, file_data, 0.0, active_sessions, transport, connection).await
    }

    /// 处理MP4文件 - 从指定位置开始的固定大小分片
    async fn process_mp4_file_from_position(
        session_id: Uuid,
        file_data: &[u8],
        start_position: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting MP4 file processing with fixed-size segmentation for session {} from position {:.3}s", session_id, start_position);
        
        // MP4文件使用固定大小分片，避免复杂的音视频分离导致的问题
        let segment_size = 256 * 1024; // 256KB per segment - 更小的分片避免stream too long
        let total_segments = (file_data.len() + segment_size - 1) / segment_size;
        
        // 注意：file_data已经是从关键帧file_offset开始的数据，所以直接从0开始分片
        info!("Starting transmission from segment 0 (position {:.3}s), {} segments remaining", 
              start_position, total_segments);
        
        // 记录传输开始时间并更新会话信息
        let transmission_start = std::time::Instant::now();
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.total_segments = total_segments as u64;
                session.transmission_start_time = Some(transmission_start);
                session.total_bytes_transmitted = 0;
            }
        }
        
        info!("MP4 file will be transmitted in {} segments of {}KB each (optimized for reliability)", 
              total_segments, segment_size / 1024);
        
        // 直接分片传输文件数据（数据已经从正确的关键帧偏移开始）
        let remaining_data = file_data;
        
        for (segment_num, chunk) in remaining_data.chunks(segment_size).enumerate() {
            if !Self::check_session_active(session_id, &active_sessions).await? {
                return Ok(());
            }
            
            // 创建MP4视频分片（从关键帧位置开始的时间戳）
            let segment_timestamp = start_position + (segment_num as f64 * 0.5); // 每个分片约0.5秒
            let segment = crate::types::Segment::Video(crate::types::VideoSegment {
                id: Uuid::new_v4(),
                data: chunk.to_vec(),
                timestamp: segment_timestamp,
                duration: 0.5, // 500ms duration per segment
                frame_count: 1,
                is_key_frame: segment_num == 0, // 第一个分片是关键帧
                metadata: crate::types::SegmentMetadata {
                    frame_indices: vec![segment_num],
                    key_frame_positions: if segment_num == 0 { vec![0] } else { vec![] },
                    encoding_params: {
                        let mut params = std::collections::HashMap::new();
                        params.insert("container".to_string(), "mp4".to_string());
                        params.insert("segment_size".to_string(), chunk.len().to_string());
                        params.insert("segment_mode".to_string(), "keyframe_aligned".to_string());
                        params.insert("start_position".to_string(), start_position.to_string());
                        params.insert("optimized_for".to_string(), "seek_accuracy".to_string());
                        params
                    },
                },
            });
            
            // 发送分片到服务器
            let segment_start = std::time::Instant::now();
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    let segment_end = std::time::Instant::now();
                    let segment_duration = segment_end.duration_since(segment_start);
                    
                    // 更新传输进度和统计信息
                    {
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.uploaded_segments = (segment_num + 1) as u64;
                            session.last_segment_time = Some(segment_end);
                            session.total_bytes_transmitted += chunk.len() as u64;
                            
                            // 计算平均分片传输时间
                            let total_time = if let Some(start) = session.transmission_start_time {
                                segment_end.duration_since(start).as_millis() as f64
                            } else {
                                segment_duration.as_millis() as f64
                            };
                            session.average_segment_time_ms = total_time / session.uploaded_segments as f64;
                            
                            // 计算当前吞吐量 (Mbps)
                            let segment_throughput = if segment_duration.as_millis() > 0 {
                                (chunk.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                            } else {
                                0.0
                            };
                            
                            if segment_throughput > session.peak_throughput_mbps {
                                session.peak_throughput_mbps = segment_throughput;
                            }
                        }
                    }
                    
                    info!("Transmitted MP4 segment {}/{} (segment {}, {:.3}s, {} bytes, {}ms, {:.1}Mbps) for session {}", 
                          segment_num + 1, total_segments, segment_num + 1,
                          segment_timestamp, chunk.len(),
                          segment_duration.as_millis(),
                          (chunk.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0,
                          session_id);
                    
                    // 极低延迟模式：最大速度传输，仅在必要时让出CPU
                    if segment_num % 50 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                Err(e) => {
                    error!("Failed to send MP4 segment {}: {}", segment_num + 1, e);
                    Self::mark_session_error(session_id, &active_sessions, e.to_string()).await;
                    return Err(UploadManagerError::TransportError(e));
                }
            }
        }
        
        // 计算并显示传输统计
        let (total_time, total_bytes, avg_time, peak_throughput) = {
            let sessions = active_sessions.read().await;
            if let Some(session) = sessions.get(&session_id) {
                let total_time = if let Some(start) = session.transmission_start_time {
                    std::time::Instant::now().duration_since(start)
                } else {
                    std::time::Duration::from_millis(0)
                };
                (total_time, session.total_bytes_transmitted, session.average_segment_time_ms, session.peak_throughput_mbps)
            } else {
                (std::time::Duration::from_millis(0), 0, 0.0, 0.0)
            }
        };
        
        let overall_throughput = if total_time.as_millis() > 0 {
            (total_bytes as f64 * 8.0) / (total_time.as_millis() as f64 / 1000.0) / 1_000_000.0
        } else {
            0.0
        };
        
        info!("MP4 file transmission completed for session {} ({} segments, {} bytes, {:.2}s, avg {:.2}ms/segment, {:.1}Mbps overall, {:.1}Mbps peak)", 
              session_id, total_segments, total_bytes, total_time.as_secs_f64(), avg_time, overall_throughput, peak_throughput);
        Ok(())
    }
    
    /// 处理通用文件 - 简单分片
    async fn process_generic_file(
        session_id: Uuid,
        file_data: &[u8],
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        Self::process_generic_file_from_position(session_id, file_data, 0.0, active_sessions, transport, connection).await
    }

    /// 处理通用文件 - 从指定位置开始的简单分片
    async fn process_generic_file_from_position(
        session_id: Uuid,
        file_data: &[u8],
        start_position: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting generic file processing for session {} from position {:.3}s", session_id, start_position);
        
        // 使用较小分片避免QUIC流限制
        let segment_size = 512 * 1024; // 512KB per segment
        let total_segments = (file_data.len() + segment_size - 1) / segment_size;
        
        // 计算起始分片索引（基于33ms每分片）
        let start_segment_index = (start_position / 0.033) as usize;
        let segments_to_transmit = if start_segment_index < total_segments {
            total_segments - start_segment_index
        } else {
            info!("Start position {:.3}s is beyond file duration, starting from beginning", start_position);
            total_segments
        };
        
        info!("Starting transmission from segment {} (position {:.3}s), {} segments remaining", 
              start_segment_index, start_position, segments_to_transmit);
        
        // 记录传输开始时间并更新会话信息
        let transmission_start = std::time::Instant::now();
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.total_segments = segments_to_transmit as u64;
                session.transmission_start_time = Some(transmission_start);
                session.total_bytes_transmitted = 0;
            }
        }
        
        info!("File will be transmitted in {} segments of {}KB each", segments_to_transmit, segment_size / 1024);
        
        // 分片传输文件数据（从指定位置开始）
        let start_byte_offset = start_segment_index * segment_size;
        let remaining_data = if start_byte_offset < file_data.len() {
            &file_data[start_byte_offset..]
        } else {
            file_data
        };
        
        for (relative_segment_num, chunk) in remaining_data.chunks(segment_size).enumerate() {
            let actual_segment_num = start_segment_index + relative_segment_num;
            if !Self::check_session_active(session_id, &active_sessions).await? {
                return Ok(());
            }
            
            // 创建通用视频分片（使用实际时间戳）
            let segment = crate::types::Segment::Video(crate::types::VideoSegment {
                id: Uuid::new_v4(),
                data: chunk.to_vec(),
                timestamp: actual_segment_num as f64 * 0.033, // 使用实际分片索引计算时间戳
                duration: 0.033,
                frame_count: 1,
                is_key_frame: actual_segment_num % 30 == 0,
                metadata: crate::types::SegmentMetadata {
                    frame_indices: vec![actual_segment_num],
                    key_frame_positions: if actual_segment_num % 30 == 0 { vec![0] } else { vec![] },
                    encoding_params: {
                        let mut params = std::collections::HashMap::new();
                        params.insert("generic_data".to_string(), "true".to_string());
                        params.insert("segment_size".to_string(), chunk.len().to_string());
                        params
                    },
                },
            });
            
            // 发送分片到服务器
            let segment_start = std::time::Instant::now();
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    let segment_end = std::time::Instant::now();
                    let segment_duration = segment_end.duration_since(segment_start);
                    
                    // 更新传输进度和统计信息
                    {
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.uploaded_segments = (relative_segment_num + 1) as u64;
                            session.last_segment_time = Some(segment_end);
                            session.total_bytes_transmitted += chunk.len() as u64;
                            
                            // 计算平均分片传输时间
                            let total_time = if let Some(start) = session.transmission_start_time {
                                segment_end.duration_since(start).as_millis() as f64
                            } else {
                                segment_duration.as_millis() as f64
                            };
                            session.average_segment_time_ms = total_time / session.uploaded_segments as f64;
                            
                            // 计算当前吞吐量 (Mbps)
                            let segment_throughput = if segment_duration.as_millis() > 0 {
                                (chunk.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                            } else {
                                0.0
                            };
                            
                            if segment_throughput > session.peak_throughput_mbps {
                                session.peak_throughput_mbps = segment_throughput;
                            }
                        }
                    }
                    
                    info!("Transmitted generic segment {}/{} (actual segment {}, {:.3}s, {} bytes, {:.2}ms, {:.1}Mbps) for session {}", 
                          relative_segment_num + 1, segments_to_transmit, actual_segment_num + 1,
                          actual_segment_num as f64 * 0.033, chunk.len(),
                          segment_duration.as_millis(),
                          (chunk.len() as f64 * 8.0) / (segment_duration.as_millis() as f64 / 1000.0) / 1_000_000.0,
                          session_id);
                    
                    // 极低延迟模式：最大速度传输，仅在必要时让出CPU
                    if relative_segment_num % 100 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                Err(e) => {
                    error!("Failed to send generic segment {} (actual segment {}): {}", relative_segment_num + 1, actual_segment_num + 1, e);
                    Self::mark_session_error(session_id, &active_sessions, e.to_string()).await;
                    return Err(UploadManagerError::TransportError(e));
                }
            }
        }
        
        // 计算并显示传输统计
        let (total_time, total_bytes, avg_time, peak_throughput) = {
            let sessions = active_sessions.read().await;
            if let Some(session) = sessions.get(&session_id) {
                let total_time = if let Some(start) = session.transmission_start_time {
                    std::time::Instant::now().duration_since(start)
                } else {
                    std::time::Duration::from_millis(0)
                };
                (total_time, session.total_bytes_transmitted, session.average_segment_time_ms, session.peak_throughput_mbps)
            } else {
                (std::time::Duration::from_millis(0), 0, 0.0, 0.0)
            }
        };
        
        let overall_throughput = if total_time.as_millis() > 0 {
            (total_bytes as f64 * 8.0) / (total_time.as_millis() as f64 / 1000.0) / 1_000_000.0
        } else {
            0.0
        };
        
        info!("Generic file transmission completed for session {} ({} segments, {} bytes, {:.2}s, avg {:.2}ms/segment, {:.1}Mbps overall, {:.1}Mbps peak)", 
              session_id, total_segments, total_bytes, total_time.as_secs_f64(), avg_time, overall_throughput, peak_throughput);
        Ok(())
    }
    
    /// 检查会话是否仍然活跃
    async fn check_session_active(
        session_id: Uuid,
        active_sessions: &Arc<RwLock<HashMap<Uuid, UploadSession>>>,
    ) -> Result<bool, UploadManagerError> {
        let sessions = active_sessions.read().await;
        if let Some(session) = sessions.get(&session_id) {
            match session.status {
                UploadStatus::Active => Ok(true),
                UploadStatus::Paused => {
                    info!("Session {} paused, waiting...", session_id);
                    drop(sessions);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    Ok(false)
                }
                UploadStatus::Completed => {
                    info!("Session {} completed, stopping transmission", session_id);
                    Ok(false)
                }
                _ => Ok(true),
            }
        } else {
            warn!("Session {} not found, stopping transmission", session_id);
            Ok(false)
        }
    }
    
    /// 启动直播流
    async fn start_live_stream(
        session_id: Uuid,
        stream_id: String,
        quality: LiveStreamQuality,
        timestamp_overlay: bool,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting live stream {} for session {}", stream_id, session_id);
        
        // 创建编码器配置
        let encoder_config = LiveEncoderConfig {
            quality: quality.clone(),
            timestamp_overlay,
            screen_capture: true, // 启用屏幕录制
            output_format: OutputFormat::H264Raw,
            segment_duration_ms: 1000 / quality.fps as u64, // 每帧一个分片
            timestamp_format: crate::live_encoder::TimestampFormat::Combined, // 组合时间戳显示
        };
        
        // 创建并启动编码器
        let mut encoder = LiveH264Encoder::new(encoder_config);
        
        match encoder.start_encoding(stream_id.clone()).await {
            Ok(_) => {
                info!("Live encoder started successfully for stream: {}", stream_id);
                
                // 启动传输任务
                tokio::spawn(async move {
                    Self::live_stream_transmission_loop(
                        session_id,
                        stream_id.clone(),
                        encoder,
                        active_sessions,
                        transport,
                        connection,
                    ).await;
                });
                
                Ok(())
            }
            Err(e) => {
                error!("Failed to start live encoder: {}", e);
                Err(UploadManagerError::TransportError(e))
            }
        }
    }
    
    /// 停止直播流
    async fn stop_live_stream(
        session_id: Uuid,
        stream_id: String,
        _active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
    ) -> Result<(), UploadManagerError> {
        info!("Stopping live stream {} for session {}", stream_id, session_id);
        
        // 这里可以添加停止编码器的逻辑
        // 由于编码器在独立的任务中运行，我们可以通过设置标志来停止它
        
        Ok(())
    }
    
    /// 直播流传输循环
    async fn live_stream_transmission_loop(
        session_id: Uuid,
        stream_id: String,
        mut encoder: LiveH264Encoder,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) {
        info!("Starting live stream transmission loop for stream: {}", stream_id);
        
        let mut frame_count = 0u64;
        let mut total_bytes = 0u64;
        let start_time = std::time::Instant::now();
        let mut last_activity = std::time::Instant::now();
        
        // 直播流运行时间限制（默认30秒，可以根据需要调整）
        let max_duration = std::time::Duration::from_secs(30);
        
        loop {
            // 检查是否超过最大运行时间
            if start_time.elapsed() > max_duration {
                info!("Live stream {} reached maximum duration, stopping", stream_id);
                break;
            }
            
            // 检查编码器状态
            let encoder_state = encoder.get_encoding_state().await;
            if !encoder_state.is_encoding {
                info!("Encoder stopped for stream {}, ending transmission", stream_id);
                break;
            }
            
            // 获取下一个编码分片
            if let Some(segment) = encoder.get_next_segment().await {
                let segment_size = match &segment {
                    crate::types::Segment::Video(v) => v.data.len(),
                    crate::types::Segment::Audio(a) => a.data.len(),
                };
                
                // 发送分片
                match transport.send_segment(&mut connection, segment).await {
                    Ok(_) => {
                        frame_count += 1;
                        total_bytes += segment_size as u64;
                        last_activity = std::time::Instant::now();
                        
                        // 每10帧打印一次统计信息（更频繁的日志）
                        if frame_count % 10 == 0 {
                            let elapsed = start_time.elapsed();
                            let fps = frame_count as f64 / elapsed.as_secs_f64();
                            let mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);
                            
                            info!("📡 LIVE STREAM {}: {} frames, {:.1} fps, {:.1} Mbps, {} bytes total", 
                                  stream_id, frame_count, fps, mbps, total_bytes);
                        }
                    }
                    Err(e) => {
                        error!("Failed to send live stream segment: {}", e);
                        // 不要立即退出，尝试继续
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            } else {
                // 没有新的分片，短暂等待
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                
                // 检查是否长时间没有新分片
                if last_activity.elapsed() > std::time::Duration::from_secs(5) {
                    warn!("No new segments for 5 seconds, checking encoder status");
                    let encoder_state = encoder.get_encoding_state().await;
                    info!("Encoder state: encoding={}, frames={}", 
                          encoder_state.is_encoding, encoder_state.frames_encoded);
                    last_activity = std::time::Instant::now(); // 重置计时器
                }
            }
        }
        
        // 停止编码器
        if let Err(e) = encoder.stop_encoding().await {
            error!("Failed to stop encoder: {}", e);
        }
        
        let elapsed = start_time.elapsed();
        let fps = frame_count as f64 / elapsed.as_secs_f64();
        let mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);
        
        info!("🏁 LIVE STREAM {} COMPLETED: {} frames in {:.2}s, avg {:.1} fps, {:.1} Mbps", 
              stream_id, frame_count, elapsed.as_secs_f64(), fps, mbps);
    }
    
    /// 标记会话错误
    async fn mark_session_error(
        session_id: Uuid,
        active_sessions: &Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        error_message: String,
    ) {
        let mut sessions = active_sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.status = UploadStatus::Error(format!("Transmission failed: {}", error_message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[tokio::test]
    async fn test_uploader_creation() {
        let uploader = OnDemandUploader::new();
        assert!(uploader.get_available_files().await.is_empty());
    }

    #[tokio::test]
    async fn test_file_registration() {
        let uploader = OnDemandUploader::new();
        
        // 创建临时测试文件
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test video data").unwrap();
        temp_file.flush().unwrap();
        
        let file_path = temp_file.path().to_path_buf();
        
        // 注册文件应该失败，因为不是有效的视频文件
        let result = uploader.register_local_file(file_path).await;
        assert!(result.is_err());
    }
}