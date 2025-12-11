use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, RwLock};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::types::{
    VideoSegment, AudioSegment, ProtocolMessage, MessageType, 
    QUICConnection, StreamType
};
use crate::errors::{TransportError, FileError, UploadManagerError};
use crate::file_reader::{FileStreamReader, DefaultFileStreamReader};
use crate::segmenter::{VideoSegmenter, DefaultVideoSegmenter};
use crate::separator::{AudioVideoSeparator, DefaultAudioVideoSeparator};
use crate::transport::{QUICTransport, DefaultQUICTransport};
use crate::controller::{PlaybackController, DefaultPlaybackController};
use crate::monitor::{PerformanceMonitor, DefaultPerformanceMonitor};
use crate::mock_server::{FileRequestPayload, PlaybackCommand};

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
    /// 活跃的上传会话
    active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
    /// 可用文件注册表
    file_registry: Arc<RwLock<HashMap<String, LocalFileInfo>>>,
    /// 控制消息接收器
    control_receiver: Option<mpsc::Receiver<PlatformMessage>>,
    /// 服务器连接
    server_connection: Option<QUICConnection>,
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
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            file_registry: Arc::new(RwLock::new(HashMap::new())),
            control_receiver: None,
            server_connection: None,
        }
    }

    /// 连接到平台服务器
    pub async fn connect_to_platform(&mut self, server_addr: std::net::SocketAddr) -> Result<(), TransportError> {
        info!("Connecting to platform server at {}", server_addr);
        
        let connection = self.transport.connect(
            server_addr,
            crate::types::QUICOptions {
                max_concurrent_streams: 100,
                initial_max_data: 10 * 1024 * 1024, // 10MB
                initial_max_stream_data: 1024 * 1024, // 1MB
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

        let file_info = LocalFileInfo {
            file_size: std::fs::metadata(&file_path)
                .map_err(|e| FileError::IoError { message: e.to_string() })?
                .len(),
            duration: Some(metadata.duration),
            format,
            available: true,
            metadata,
            file_path: file_path.clone(),
        };

        let file_key = file_path.to_string_lossy().to_string();
        self.file_registry.write().await.insert(file_key.clone(), file_info);
        
        info!("Successfully registered file: {}", file_key);
        Ok(())
    }

    /// 获取可用文件列表
    pub async fn get_available_files(&self) -> Vec<String> {
        self.file_registry.read().await.keys().cloned().collect()
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
        let data = recv_stream.read_to_end(1024 * 1024).await // 1MB limit
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
                            session.current_position = position;
                            
                            let mut controller_guard = controller.lock().await;
                            if let Err(e) = controller_guard.seek(position).await {
                                error!("Seek error: {}", e);
                                session.status = UploadStatus::Error(e.to_string());
                            } else {
                                session.status = UploadStatus::Active;
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

        // 如果有seek位置，先定位
        if let Some(position) = seek_position {
            let mut controller_guard = controller.lock().await;
            controller_guard.seek(position).await
                .map_err(|e| UploadManagerError::PlaybackError(e))?;
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

            // 极低延迟传输时间（根据播放速率调整）
            let sessions = active_sessions.read().await;
            let transmission_delay = if let Some(session) = sessions.get(&session_id) {
                // 基础传输时间33ms（对应30fps），根据播放速率调整
                let base_delay = 33.0; // 33ms for 30fps low latency
                let adjusted_delay = base_delay / session.playback_rate;
                // 最小延迟不低于10ms，最大不超过100ms
                let clamped_delay = adjusted_delay.clamp(10.0, 100.0);
                std::time::Duration::from_millis(clamped_delay as u64)
            } else {
                std::time::Duration::from_millis(33) // Default 33ms for 30fps
            };
            drop(sessions);

            // 实际发送分片数据到服务器
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    // 模拟传输延迟
                    tokio::time::sleep(transmission_delay).await;

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

    /// 真实文件传输过程
    async fn real_file_transmission(
        session_id: Uuid,
        file_info: &LocalFileInfo,
        mut file_handle: tokio::fs::File,
        seek_position: Option<f64>,
        playback_rate: f64,
        active_sessions: Arc<RwLock<HashMap<Uuid, UploadSession>>>,
        file_reader: Arc<DefaultFileStreamReader>,
        transport: Arc<DefaultQUICTransport>,
        mut connection: QUICConnection,
    ) -> Result<(), UploadManagerError> {
        info!("Starting real file transmission for session {}", session_id);
        
        // 读取整个文件内容
        let file_data = std::fs::read(&file_info.file_path)
            .map_err(|e| UploadManagerError::FileError(
                crate::errors::FileError::IoError { message: e.to_string() }
            ))?;
        
        info!("Read {} bytes from file: {:?}", file_data.len(), file_info.file_path);
        
        // 计算分片大小和数量
        let segment_size = 8192; // 8KB per segment
        let total_segments = (file_data.len() + segment_size - 1) / segment_size;
        
        // 更新会话的总分片数
        {
            let mut sessions = active_sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                session.total_segments = total_segments as u64;
            }
        }
        
        info!("File will be transmitted in {} segments of {}KB each", total_segments, segment_size / 1024);
        
        // 分片传输文件数据
        for (segment_num, chunk) in file_data.chunks(segment_size).enumerate() {
            // 检查会话状态
            let should_continue = {
                let sessions = active_sessions.read().await;
                if let Some(session) = sessions.get(&session_id) {
                    match session.status {
                        UploadStatus::Active => true,
                        UploadStatus::Paused => {
                            info!("Session {} paused, waiting...", session_id);
                            false
                        }
                        UploadStatus::Completed => {
                            info!("Session {} completed, stopping transmission", session_id);
                            return Ok(());
                        }
                        _ => true,
                    }
                } else {
                    warn!("Session {} not found, stopping transmission", session_id);
                    return Ok(());
                }
            };
            
            if !should_continue {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
            
            // 创建真实的视频分片
            let segment = crate::types::Segment::Video(crate::types::VideoSegment {
                id: Uuid::new_v4(),
                data: chunk.to_vec(), // 真实的文件数据
                timestamp: segment_num as f64 * 0.033, // 33ms per segment
                duration: 0.033,
                frame_count: 1,
                is_key_frame: segment_num % 30 == 0, // 每30个分片一个关键帧
                metadata: crate::types::SegmentMetadata {
                    frame_indices: vec![0],
                    key_frame_positions: if segment_num % 30 == 0 { vec![0] } else { vec![] },
                    encoding_params: {
                        let mut params = std::collections::HashMap::new();
                        params.insert("real_data".to_string(), "true".to_string());
                        params.insert("segment_size".to_string(), chunk.len().to_string());
                        params
                    },
                },
            });
            
            // 发送分片到服务器
            match transport.send_segment(&mut connection, segment).await {
                Ok(_) => {
                    // 更新传输进度
                    {
                        let mut sessions = active_sessions.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            session.uploaded_segments = (segment_num + 1) as u64;
                        }
                    }
                    
                    info!("Transmitted real data segment {}/{} ({} bytes) for session {}", 
                          segment_num + 1, total_segments, chunk.len(), session_id);
                    
                    // 传输延迟（根据播放速率调整）
                    let delay_ms = (33.0 / playback_rate).clamp(10.0, 100.0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
                Err(e) => {
                    error!("Failed to send real data segment {}: {}", segment_num + 1, e);
                    // 更新会话状态为错误
                    let mut sessions = active_sessions.write().await;
                    if let Some(session) = sessions.get_mut(&session_id) {
                        session.status = UploadStatus::Error(format!("Transmission failed: {}", e));
                    }
                    return Err(UploadManagerError::TransportError(e));
                }
            }
        }
        
        info!("Real file transmission completed for session {} ({} segments, {} bytes)", 
              session_id, total_segments, file_data.len());
        Ok(())
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