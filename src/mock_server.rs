use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tokio::fs::create_dir_all;
use tokio::io::AsyncWriteExt;
use quinn::{ServerConfig, Endpoint, Connection, RecvStream, SendStream};
use rustls::{Certificate, PrivateKey, ServerConfig as TlsServerConfig};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::types::{
    VideoSegment, AudioSegment, ProtocolMessage, MessageType, 
    StatusCode, StreamType, QUICConnection
};
use crate::errors::TransportError;

/// 模拟平台服务器，用于接收和处理视频流上传
pub struct MockPlatformServer {
    endpoint: Option<Endpoint>,
    sessions: Arc<Mutex<HashMap<Uuid, ServerSession>>>,
    file_registry: Arc<Mutex<HashMap<String, FileInfo>>>,
    config: ServerConfig,
}

/// 服务器会话信息
#[derive(Debug, Clone)]
pub struct ServerSession {
    pub id: Uuid,
    pub connection: QUICConnection,
    pub requested_files: Vec<String>,
    pub active_streams: HashMap<StreamType, u64>,
    pub received_segments: Vec<ReceivedSegment>,
    pub status: SessionStatus,
    /// 当前接收的文件信息
    pub current_file: Option<ReceivingFileInfo>,
    /// 活跃的直播流
    pub live_streams: HashMap<String, LiveStreamInfo>,
}

/// 直播流信息
#[derive(Debug, Clone)]
pub struct LiveStreamInfo {
    pub stream_id: String,
    pub output_path: std::path::PathBuf,
    pub start_time: std::time::SystemTime,
    pub frames_received: u64,
    pub bytes_received: u64,
    pub is_active: bool,
}

/// 正在接收的文件信息
#[derive(Debug, Clone)]
pub struct ReceivingFileInfo {
    pub file_path: String,
    pub output_path: std::path::PathBuf,
    pub total_size: u64,
    pub received_size: u64,
    pub video_segments: Vec<ReceivedSegment>,
    pub audio_segments: Vec<ReceivedSegment>,
    pub start_time: std::time::SystemTime,
    // 新增：接收端性能统计
    pub reception_start_time: Option<std::time::Instant>,
    pub last_segment_time: Option<std::time::Instant>,
    pub total_segments_received: u64,
    pub average_receive_time_ms: f64,
    pub peak_receive_throughput_mbps: f64,
    // 新增：直播流相关
    pub is_live_stream: bool,
    pub live_stream_id: Option<String>,
}

/// 会话状态
#[derive(Debug, Clone)]
pub enum SessionStatus {
    Connected,
    Requesting,
    Receiving,
    Paused,
    Completed,
    Error(String),
}

/// 接收到的分片信息
#[derive(Debug, Clone)]
pub struct ReceivedSegment {
    pub segment_id: Uuid,
    pub segment_type: StreamType,
    pub timestamp: f64,
    pub data_size: usize,
    pub received_at: std::time::SystemTime,
}

/// 文件信息注册表
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub file_path: String,
    pub file_size: u64,
    pub duration: f64,
    pub available: bool,
    pub priority: u8,
}

impl MockPlatformServer {
    /// 创建新的模拟服务器
    pub fn new() -> Result<Self, TransportError> {
        let config = Self::create_server_config()?;
        
        Ok(Self {
            endpoint: None,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            file_registry: Arc::new(Mutex::new(HashMap::new())),
            config,
        })
    }

    /// 启动服务器监听
    pub async fn start(&mut self, bind_addr: SocketAddr) -> Result<(), TransportError> {
        let endpoint = Endpoint::server(self.config.clone(), bind_addr)
            .map_err(|e| TransportError::ConnectionFailed { reason: e.to_string() })?;
        
        info!("Mock platform server started on {}", bind_addr);
        self.endpoint = Some(endpoint.clone());

        // 启动连接处理循环
        let sessions = self.sessions.clone();
        let file_registry = self.file_registry.clone();
        
        tokio::spawn(async move {
            Self::handle_connections(endpoint, sessions, file_registry).await;
        });

        Ok(())
    }

    /// 注册可用文件
    pub async fn register_file(&self, file_path: String, info: FileInfo) {
        let mut registry = self.file_registry.lock().await;
        let file_path_clone = file_path.clone();
        registry.insert(file_path, info);
        info!("Registered file for streaming: {}", file_path_clone);
    }

    /// 请求客户端上传指定文件
    pub async fn request_file_upload(&self, session_id: Uuid, file_path: String) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get_mut(&session_id) {
            // 发送文件请求消息
            let request_msg = ProtocolMessage {
                message_type: MessageType::FileRequest,
                session_id,
                timestamp: std::time::SystemTime::now(),
                sequence_number: 1,
                payload: serde_json::to_vec(&FileRequestPayload {
                    file_path: file_path.clone(),
                    priority: 1,
                    seek_position: None,
                    playback_rate: 1.0,
                }).map_err(|e| TransportError::SerializationError { message: e.to_string() })?,
            };

            // 通过QUIC连接发送消息到客户端
            let mut send_stream = session.connection.inner.open_uni().await
                .map_err(|e| TransportError::StreamCreationFailed { 
                    reason: format!("Failed to open request stream: {}", e) 
                })?;
                
            let serialized = serde_json::to_vec(&request_msg)
                .map_err(|e| TransportError::SerializationError { 
                    message: format!("Failed to serialize request: {}", e) 
                })?;
                
            send_stream.write_all(&serialized).await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to send request: {}", e) 
                })?;
                
            send_stream.finish().await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to finish request stream: {}", e) 
                })?;

            session.requested_files.push(file_path.clone());
            session.status = SessionStatus::Requesting;
            
            // 初始化文件接收信息
            let output_dir = PathBuf::from("recv_videos");
            let file_name = PathBuf::from(&file_path)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("unknown"))
                .to_string_lossy()
                .to_string();
            let output_path = output_dir.join(format!("{}_{}.recv", session_id, file_name));
            
            session.current_file = Some(ReceivingFileInfo {
                file_path: file_path.clone(),
                output_path,
                total_size: 0,
                received_size: 0,
                video_segments: Vec::new(),
                audio_segments: Vec::new(),
                start_time: std::time::SystemTime::now(),
                // 初始化接收统计
                reception_start_time: None,
                last_segment_time: None,
                total_segments_received: 0,
                average_receive_time_ms: 0.0,
                peak_receive_throughput_mbps: 0.0,
                // 初始化直播流字段
                is_live_stream: false,
                live_stream_id: None,
            });
            
            info!("Requested file upload: {} from session {}", file_path, session_id);
            info!("Will save received data to: {:?}", session.current_file.as_ref().unwrap().output_path);
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { reason: format!("Session {} not found", session_id) })
        }
    }

    /// 发送播放控制命令
    pub async fn send_playback_control(
        &self, 
        session_id: Uuid, 
        command: PlaybackCommand
    ) -> Result<(), TransportError> {
        let sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get(&session_id) {
            let control_msg = ProtocolMessage {
                message_type: MessageType::PlaybackControl,
                session_id,
                timestamp: std::time::SystemTime::now(),
                sequence_number: 1,
                payload: serde_json::to_vec(&command)
                    .map_err(|e| TransportError::SerializationError { message: e.to_string() })?,
            };

            // 通过QUIC连接发送播放控制消息到客户端
            let mut send_stream = session.connection.inner.open_uni().await
                .map_err(|e| TransportError::StreamCreationFailed { 
                    reason: format!("Failed to open control stream: {}", e) 
                })?;
                
            let serialized = serde_json::to_vec(&control_msg)
                .map_err(|e| TransportError::SerializationError { 
                    message: format!("Failed to serialize control message: {}", e) 
                })?;
                
            send_stream.write_all(&serialized).await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to send control message: {}", e) 
                })?;
                
            send_stream.finish().await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to finish control stream: {}", e) 
                })?;

            info!("Sent playback control: {:?} to session {}", command, session_id);
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { reason: format!("Session {} not found", session_id) })
        }
    }

    /// 发送直播流控制命令
    pub async fn send_live_stream_control(
        &self, 
        session_id: Uuid, 
        command: PlaybackCommand
    ) -> Result<(), TransportError> {
        // 根据命令类型进行预处理
        match &command {
            PlaybackCommand::StartLive { stream_id, .. } => {
                // 启动直播流接收
                self.start_live_stream_reception(session_id, stream_id.clone()).await?;
            }
            PlaybackCommand::StopLive { stream_id } => {
                // 停止直播流接收
                self.stop_live_stream_reception(session_id, stream_id.clone()).await?;
            }
            _ => {}
        }
        
        let sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get(&session_id) {
            let control_msg = ProtocolMessage {
                message_type: MessageType::LiveStreamControl,
                session_id,
                timestamp: std::time::SystemTime::now(),
                sequence_number: 1,
                payload: serde_json::to_vec(&command)
                    .map_err(|e| TransportError::SerializationError { message: e.to_string() })?,
            };

            // 通过QUIC连接发送直播控制消息到客户端
            let mut send_stream = session.connection.inner.open_uni().await
                .map_err(|e| TransportError::StreamCreationFailed { 
                    reason: format!("Failed to open live control stream: {}", e) 
                })?;
                
            let serialized = serde_json::to_vec(&control_msg)
                .map_err(|e| TransportError::SerializationError { 
                    message: format!("Failed to serialize live control message: {}", e) 
                })?;
                
            send_stream.write_all(&serialized).await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to send live control message: {}", e) 
                })?;
                
            send_stream.finish().await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to finish live control stream: {}", e) 
                })?;

            info!("Sent live stream control: {:?} to session {}", command, session_id);
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { reason: format!("Session {} not found", session_id) })
        }
    }

    /// 查询客户端可用文件列表
    pub async fn query_client_files(&self, session_id: Uuid, filter: Option<String>) -> Result<(), TransportError> {
        let sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get(&session_id) {
            let query_msg = ProtocolMessage {
                message_type: MessageType::FileListQuery,
                session_id,
                timestamp: std::time::SystemTime::now(),
                sequence_number: 1,
                payload: serde_json::to_vec(&FileListQuery { filter })
                    .map_err(|e| TransportError::SerializationError { message: e.to_string() })?,
            };

            // 发送查询消息到客户端
            let mut send_stream = session.connection.inner.open_uni().await
                .map_err(|e| TransportError::StreamCreationFailed { 
                    reason: format!("Failed to open query stream: {}", e) 
                })?;
                
            let serialized = serde_json::to_vec(&query_msg)
                .map_err(|e| TransportError::SerializationError { 
                    message: format!("Failed to serialize query: {}", e) 
                })?;
                
            send_stream.write_all(&serialized).await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to send query: {}", e) 
                })?;
                
            send_stream.finish().await
                .map_err(|e| TransportError::NetworkError { 
                    message: format!("Failed to finish query stream: {}", e) 
                })?;

            info!("Querying file list from session {}", session_id);
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { reason: format!("Session {} not found", session_id) })
        }
    }

    /// 获取会话统计信息
    pub async fn get_session_stats(&self, session_id: Uuid) -> Option<SessionStats> {
        let sessions = self.sessions.lock().await;
        
        sessions.get(&session_id).map(|session| {
            let video_segments = session.received_segments.iter()
                .filter(|s| matches!(s.segment_type, StreamType::Video))
                .count();
            let audio_segments = session.received_segments.iter()
                .filter(|s| matches!(s.segment_type, StreamType::Audio))
                .count();
            
            SessionStats {
                session_id,
                status: session.status.clone(),
                total_segments: session.received_segments.len(),
                video_segments,
                audio_segments,
                requested_files: session.requested_files.clone(),
                current_file: session.current_file.as_ref().map(|f| f.file_path.clone()),
                received_size: session.current_file.as_ref().map(|f| f.received_size).unwrap_or(0),
                output_path: session.current_file.as_ref().map(|f| f.output_path.clone()),
            }
        })
    }

    /// 获取所有活跃会话
    pub async fn get_all_sessions(&self) -> Vec<(Uuid, SessionStatus)> {
        let sessions = self.sessions.lock().await;
        sessions.iter()
            .map(|(id, session)| (*id, session.status.clone()))
            .collect()
    }

    /// 处理传入连接
    async fn handle_connections(
        endpoint: Endpoint,
        sessions: Arc<Mutex<HashMap<Uuid, ServerSession>>>,
        file_registry: Arc<Mutex<HashMap<String, FileInfo>>>,
    ) {
        while let Some(conn) = endpoint.accept().await {
            let sessions = sessions.clone();
            let file_registry = file_registry.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(conn, sessions, file_registry).await {
                    error!("Connection handling error: {}", e);
                }
            });
        }
    }

    /// 处理单个连接
    async fn handle_connection(
        connecting: quinn::Connecting,
        sessions: Arc<Mutex<HashMap<Uuid, ServerSession>>>,
        _file_registry: Arc<Mutex<HashMap<String, FileInfo>>>,
    ) -> Result<(), TransportError> {
        let connection = connecting.await
            .map_err(|e| TransportError::ConnectionFailed { reason: e.to_string() })?;
        
        let session_id = Uuid::new_v4();
        info!("New client connection established: {}", session_id);

        // 创建会话
        let session = ServerSession {
            id: session_id,
            connection: QUICConnection {
                id: session_id,
                remote_address: connection.remote_address(),
                established_at: std::time::SystemTime::now(),
                stats: crate::types::ConnectionStats {
                    rtt: std::time::Duration::from_millis(10),
                    bandwidth: 10_000_000, // 10 Mbps
                    packet_loss: 0.0,
                    congestion_window: 65536,
                },
                inner: Arc::new(connection.clone()),
                config: crate::types::QUICOptions {
                    max_concurrent_streams: 100,
                    initial_max_data: 50 * 1024 * 1024, // 50MB - increased for large file transfers
                    initial_max_stream_data: 5 * 1024 * 1024, // 5MB - increased to support 1MB+ segments
                    idle_timeout: std::time::Duration::from_secs(30),
                },
            },
            requested_files: Vec::new(),
            active_streams: HashMap::new(),
            received_segments: Vec::new(),
            status: SessionStatus::Connected,
            current_file: None,
            live_streams: HashMap::new(),
        };

        sessions.lock().await.insert(session_id, session);

        // 处理连接上的流
        loop {
            match connection.accept_uni().await {
                Ok(recv_stream) => {
                    let sessions = sessions.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_stream(session_id, recv_stream, sessions).await {
                            error!("Stream handling error: {}", e);
                        }
                    });
                }
                Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                    info!("Client {} disconnected", session_id);
                    break;
                }
                Err(e) => {
                    error!("Connection error: {}", e);
                    break;
                }
            }
        }

        // 清理会话
        sessions.lock().await.remove(&session_id);
        Ok(())
    }

    /// 处理数据流
    async fn handle_stream(
        session_id: Uuid,
        mut recv_stream: RecvStream,
        sessions: Arc<Mutex<HashMap<Uuid, ServerSession>>>,
    ) -> Result<(), TransportError> {
        // 记录接收开始时间
        let receive_start = std::time::Instant::now();
        
        // 读取流数据 - 增加限制以支持大分片传输
        let data = recv_stream.read_to_end(2 * 1024 * 1024) // 2MB limit to accommodate 1MB segments + headers
            .await
            .map_err(|e| TransportError::NetworkError { message: e.to_string() })?;
            
        let receive_end = std::time::Instant::now();
        let receive_duration = receive_end.duration_since(receive_start);

        // 尝试解析为协议消息
        if let Ok(protocol_msg) = serde_json::from_slice::<ProtocolMessage>(&data) {
            info!("Received protocol message: {:?} from session {}", protocol_msg.message_type, session_id);
            
            match protocol_msg.message_type {
                MessageType::SessionStart => {
                    info!("Client handshake received for session {}", session_id);
                    // 更新会话状态为已连接
                    let mut sessions_guard = sessions.lock().await;
                    if let Some(session) = sessions_guard.get_mut(&session_id) {
                        session.status = SessionStatus::Connected;
                    }
                }
                MessageType::FileListResponse => {
                    if let Ok(response) = serde_json::from_slice::<FileListResponse>(&protocol_msg.payload) {
                        info!("Received file list response with {} files", response.files.len());
                        for file in &response.files {
                            info!("  - {} ({} bytes, {})", file.file_path, file.file_size, file.format);
                        }
                    }
                }
                _ => {
                    info!("Received other protocol message: {:?}", protocol_msg.message_type);
                }
            }
            return Ok(());
        }

        // 如果不是协议消息，尝试解析为分片数据
        if data.len() < 8 {
            return Err(TransportError::InvalidData { message: "Stream too short".to_string() });
        }

        let stream_type_id = data[0];
        let stream_type = match stream_type_id {
            0x01 => StreamType::Video,
            0x02 => StreamType::Audio,
            _ => return Err(TransportError::InvalidData { message: "Unknown stream type".to_string() }),
        };

        // 解析分片数据
        let segment = Self::parse_segment_data(&data[1..], stream_type)?;
        
        // 记录接收到的分片并写入文件
        let mut sessions_guard = sessions.lock().await;
        if let Some(session) = sessions_guard.get_mut(&session_id) {
            let received_segment = ReceivedSegment {
                segment_id: segment.id,
                segment_type: stream_type,
                timestamp: segment.timestamp,
                data_size: segment.data.len(),
                received_at: std::time::SystemTime::now(),
            };
            
            session.received_segments.push(received_segment.clone());
            
            // 写入文件数据
            if let Some(ref mut file_info) = session.current_file {
                // 初始化接收开始时间
                if file_info.reception_start_time.is_none() {
                    file_info.reception_start_time = Some(receive_start);
                }
                
                // 更新接收统计
                file_info.received_size += segment.data.len() as u64;
                file_info.last_segment_time = Some(receive_end);
                file_info.total_segments_received += 1;
                
                // 计算平均接收时间
                let total_time = if let Some(start) = file_info.reception_start_time {
                    receive_end.duration_since(start).as_millis() as f64
                } else {
                    receive_duration.as_millis() as f64
                };
                file_info.average_receive_time_ms = total_time / file_info.total_segments_received as f64;
                
                // 计算当前分片的接收吞吐量
                let segment_throughput = if receive_duration.as_millis() > 0 {
                    (segment.data.len() as f64 * 8.0) / (receive_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                } else {
                    0.0
                };
                
                if segment_throughput > file_info.peak_receive_throughput_mbps {
                    file_info.peak_receive_throughput_mbps = segment_throughput;
                }
                
                // 检查是否是直播流数据
                let is_live_stream = file_info.is_live_stream;
                let live_stream_id = file_info.live_stream_id.clone();
                
                // 分类存储分片
                match stream_type {
                    StreamType::Video => file_info.video_segments.push(received_segment.clone()),
                    StreamType::Audio => file_info.audio_segments.push(received_segment.clone()),
                }
                
                // 如果是直播流，更新直播流统计
                if is_live_stream {
                    if let Some(stream_id) = &live_stream_id {
                        if let Some(live_stream) = session.live_streams.get_mut(stream_id) {
                            live_stream.frames_received += 1;
                            live_stream.bytes_received += segment.data.len() as u64;
                        }
                    }
                }
                
                // 异步写入文件
                let output_path = file_info.output_path.clone();
                let segment_data = segment.data.clone();
                let segment_timestamp = segment.timestamp;
                let segment_type_name = match stream_type {
                    StreamType::Video => "video",
                    StreamType::Audio => "audio",
                };
                
                // 释放锁后再进行文件操作
                drop(sessions_guard);
                
                // 写入分片数据到文件
                if let Err(e) = Self::write_segment_to_file(
                    &output_path,
                    &segment_data,
                    segment_timestamp,
                    stream_type,
                    receive_duration,
                ).await {
                    error!("Failed to write segment to file: {}", e);
                } else {
                    // 计算接收吞吐量
                    let throughput_mbps = if receive_duration.as_millis() > 0 {
                        (segment_data.len() as f64 * 8.0) / (receive_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                    } else {
                        0.0
                    };
                    
                    // 根据是否是直播流显示不同的日志
                    if is_live_stream {
                        if let Some(_stream_id) = &live_stream_id {
                            println!("📺 LIVE RECEIVE: {}ms - {} segment {:.3}s ({} bytes, {:.1}Mbps) -> {:?}", 
                                   std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                                       .unwrap_or_default().as_millis(),
                                   segment_type_name, segment_timestamp, segment_data.len(), throughput_mbps, output_path);
                        }
                    } else {
                        info!("Received {} segment {:.3}s ({} bytes, {:.2}ms, {:.1}Mbps) to {:?}", 
                              segment_type_name, segment_timestamp, segment_data.len(), 
                              receive_duration.as_millis(), throughput_mbps, output_path);
                    }
                }
            } else {
                drop(sessions_guard);
                debug!("Received {} segment {} from session {} (no active file)", 
                       match stream_type {
                           StreamType::Video => "video",
                           StreamType::Audio => "audio",
                       },
                       segment.id, 
                       session_id);
            }
        }

        Ok(())
    }

    /// 解析分片数据
    fn parse_segment_data(data: &[u8], stream_type: StreamType) -> Result<SegmentData, TransportError> {
        match stream_type {
            StreamType::Video => {
                // 视频分片格式: ID(16) + 时间戳(8) + 持续时间(8) + 帧数(4) + 标志(1) + 数据长度(4) + 数据
                if data.len() < 41 {
                    return Err(TransportError::InvalidData { message: "Video segment data too short".to_string() });
                }

                // 解析分片ID (16字节)
                let mut id_bytes = [0u8; 16];
                id_bytes.copy_from_slice(&data[0..16]);
                let id = Uuid::from_bytes(id_bytes);

                // 解析时间戳 (8字节)
                let timestamp = f64::from_be_bytes([
                    data[16], data[17], data[18], data[19],
                    data[20], data[21], data[22], data[23],
                ]);

                // 解析持续时间 (8字节)
                let duration = f64::from_be_bytes([
                    data[24], data[25], data[26], data[27],
                    data[28], data[29], data[30], data[31],
                ]);

                // 跳过帧数(4字节) + 标志(1字节)
                // 解析数据长度 (4字节)
                let data_length = u32::from_be_bytes([
                    data[37], data[38], data[39], data[40],
                ]) as usize;

                // 提取实际视频数据
                let start_pos = 41;
                if data.len() < start_pos + data_length {
                    return Err(TransportError::InvalidData { message: "Video data length mismatch".to_string() });
                }
                let segment_data = data[start_pos..start_pos + data_length].to_vec();

                Ok(SegmentData {
                    id,
                    timestamp,
                    duration,
                    data: segment_data,
                })
            }
            StreamType::Audio => {
                // 音频分片格式: ID(16) + 时间戳(8) + 持续时间(8) + 采样率(4) + 声道(2) + 数据长度(4) + 数据
                if data.len() < 42 {
                    return Err(TransportError::InvalidData { message: "Audio segment data too short".to_string() });
                }

                // 解析分片ID (16字节)
                let mut id_bytes = [0u8; 16];
                id_bytes.copy_from_slice(&data[0..16]);
                let id = Uuid::from_bytes(id_bytes);

                // 解析时间戳 (8字节)
                let timestamp = f64::from_be_bytes([
                    data[16], data[17], data[18], data[19],
                    data[20], data[21], data[22], data[23],
                ]);

                // 解析持续时间 (8字节)
                let duration = f64::from_be_bytes([
                    data[24], data[25], data[26], data[27],
                    data[28], data[29], data[30], data[31],
                ]);

                // 跳过采样率(4字节) + 声道数(2字节)
                // 解析数据长度 (4字节)
                let data_length = u32::from_be_bytes([
                    data[38], data[39], data[40], data[41],
                ]) as usize;

                // 提取实际音频数据
                let start_pos = 42;
                if data.len() < start_pos + data_length {
                    return Err(TransportError::InvalidData { message: "Audio data length mismatch".to_string() });
                }
                let segment_data = data[start_pos..start_pos + data_length].to_vec();

                Ok(SegmentData {
                    id,
                    timestamp,
                    duration,
                    data: segment_data,
                })
            }
        }
    }

    /// 写入分片数据到文件
    async fn write_segment_to_file(
        output_path: &PathBuf,
        segment_data: &[u8],
        timestamp: f64,
        stream_type: StreamType,
        receive_duration: std::time::Duration,
    ) -> Result<(), std::io::Error> {
        // 确保输出目录存在
        if let Some(parent) = output_path.parent() {
            create_dir_all(parent).await?;
        }
        
        // 创建两个文件：原始数据文件和调试信息文件
        // 保持原始文件扩展名，不要强制改为.mp4
        let raw_output_path = if output_path.extension().and_then(|s| s.to_str()) == Some("recv") {
            // 如果是.recv文件，去掉.recv后缀，保留原始扩展名
            let file_stem = output_path.file_stem().unwrap_or_default().to_string_lossy();
            if file_stem.ends_with(".h264") {
                output_path.with_file_name(format!("{}", file_stem))
            } else if file_stem.ends_with(".mp4") {
                output_path.with_file_name(format!("{}", file_stem))
            } else {
                output_path.with_extension("mp4") // 默认情况
            }
        } else {
            output_path.clone()
        };
        let debug_output_path = output_path.with_extension("debug"); // 调试信息文件
        
        // 写入原始数据（可播放的视频文件）
        let mut raw_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&raw_output_path)
            .await?;
        raw_file.write_all(segment_data).await?;
        raw_file.flush().await?;
        
        // 写入调试信息（用于验证传输）
        let throughput_mbps = if receive_duration.as_millis() > 0 {
            (segment_data.len() as f64 * 8.0) / (receive_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
        } else {
            0.0
        };
        
        let debug_header = format!(
            "SEGMENT|{}|{:.6}|{}|{:.2}ms|{:.1}Mbps|{:?}\n",
            match stream_type {
                StreamType::Video => "VIDEO",
                StreamType::Audio => "AUDIO",
            },
            timestamp,
            segment_data.len(),
            receive_duration.as_millis(),
            throughput_mbps,
            std::time::SystemTime::now()
        );
        
        let mut debug_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&debug_output_path)
            .await?;
        debug_file.write_all(debug_header.as_bytes()).await?;
        debug_file.write_all(b"---SEGMENT_END---\n").await?;
        debug_file.flush().await?;
        
        Ok(())
    }
    
    /// 完成文件接收并生成统计报告
    pub async fn finalize_file_reception(&self, session_id: Uuid) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get_mut(&session_id) {
            if let Some(file_info) = &session.current_file {
                let duration = file_info.start_time.elapsed().unwrap_or_default();
                let reception_duration = if let Some(start) = file_info.reception_start_time {
                    std::time::Instant::now().duration_since(start)
                } else {
                    std::time::Duration::from_millis(0)
                };
                let output_path = &file_info.output_path;
                
                // 生成统计报告
                let report_path = output_path.with_extension("report");
                // 保持原始文件扩展名
                let raw_video_path = if output_path.extension().and_then(|s| s.to_str()) == Some("recv") {
                    let file_stem = output_path.file_stem().unwrap_or_default().to_string_lossy();
                    if file_stem.ends_with(".h264") {
                        output_path.with_file_name(format!("{}", file_stem))
                    } else if file_stem.ends_with(".mp4") {
                        output_path.with_file_name(format!("{}", file_stem))
                    } else {
                        output_path.with_extension("mp4")
                    }
                } else {
                    output_path.clone()
                };
                let debug_info_path = output_path.with_extension("debug");
                
                // 计算接收性能统计
                let overall_receive_throughput = if reception_duration.as_millis() > 0 {
                    (file_info.received_size as f64 * 8.0) / (reception_duration.as_millis() as f64 / 1000.0) / 1_000_000.0
                } else {
                    0.0
                };
                
                let report_content = format!(
                    "=== 文件接收完成报告 ===\n\
                     原始文件: {}\n\
                     会话ID: {}\n\
                     接收开始时间: {:?}\n\
                     总耗时: {:.2}秒\n\
                     实际接收耗时: {:.2}秒\n\
                     总接收数据: {} bytes\n\
                     视频分片数: {}\n\
                     音频分片数: {}\n\
                     总分片数: {}\n\
                     \n\
                     输出文件:\n\
                     - 可播放视频文件: {:?}\n\
                     - 传输调试信息: {:?}\n\
                     - 统计报告文件: {:?}\n\
                     \n\
                     接收性能统计:\n\
                     - 总体接收速率: {:.1} Mbps\n\
                     - 峰值接收速率: {:.1} Mbps\n\
                     - 平均分片接收时间: {:.2} ms\n\
                     - 平均分片大小: {:.1} KB\n\
                     - 接收效率: {:.1}%\n\
                     \n\
                     说明:\n\
                     - .mp4 文件包含原始视频数据，可直接播放\n\
                     - .debug 文件包含每个分片的详细接收统计\n\
                     - .report 文件为本统计报告\n\
                     =========================\n",
                    file_info.file_path,
                    session_id,
                    file_info.start_time,
                    duration.as_secs_f64(),
                    reception_duration.as_secs_f64(),
                    file_info.received_size,
                    file_info.video_segments.len(),
                    file_info.audio_segments.len(),
                    file_info.total_segments_received,
                    raw_video_path,
                    debug_info_path,
                    report_path,
                    overall_receive_throughput,
                    file_info.peak_receive_throughput_mbps,
                    file_info.average_receive_time_ms,
                    if file_info.total_segments_received > 0 {
                        file_info.received_size as f64 / 1024.0 / file_info.total_segments_received as f64
                    } else {
                        0.0
                    },
                    if duration.as_secs_f64() > 0.0 {
                        (reception_duration.as_secs_f64() / duration.as_secs_f64()) * 100.0
                    } else {
                        0.0
                    }
                );
                
                tokio::fs::write(&report_path, report_content).await
                    .map_err(|e| TransportError::NetworkError { message: e.to_string() })?;
                
                info!("File reception completed for session {}", session_id);
                info!("  Original file: {}", file_info.file_path);
                info!("  Received: {} bytes in {:.2}s (actual reception: {:.2}s)", 
                      file_info.received_size, duration.as_secs_f64(), reception_duration.as_secs_f64());
                info!("  Segments: {} total ({} video, {} audio)", 
                      file_info.total_segments_received, file_info.video_segments.len(), file_info.audio_segments.len());
                info!("  Performance: {:.1}Mbps overall, {:.1}Mbps peak, avg {:.2}ms/segment", 
                      overall_receive_throughput, file_info.peak_receive_throughput_mbps, file_info.average_receive_time_ms);
                info!("  Files: {:?} (video), {:?} (debug), {:?} (report)", 
                      raw_video_path, output_path.with_extension("debug"), report_path);
                
                // 清理当前文件信息
                session.current_file = None;
                session.status = SessionStatus::Completed;
            }
        }
        
        Ok(())
    }

    /// 创建服务器配置
    fn create_server_config() -> Result<ServerConfig, TransportError> {
        // 创建自签名证书用于测试
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .map_err(|e| TransportError::ConfigurationError { message: e.to_string() })?;
        
        let cert_der = cert.serialize_der()
            .map_err(|e| TransportError::ConfigurationError { message: e.to_string() })?;
        let priv_key = cert.serialize_private_key_der();

        let mut tls_config = TlsServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(
                vec![Certificate(cert_der)],
                PrivateKey(priv_key),
            )
            .map_err(|e| TransportError::ConfigurationError { message: e.to_string() })?;

        tls_config.alpn_protocols = vec![b"video-streaming/1.0".to_vec()];

        let mut server_config = ServerConfig::with_crypto(Arc::new(tls_config));
        let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
        
        // 配置传输参数 - 增加流数据限制以支持大分片传输
        transport_config.max_concurrent_uni_streams(100_u32.into());
        transport_config.max_concurrent_bidi_streams(10_u32.into());
        transport_config.stream_receive_window((5 * 1024 * 1024u32).try_into().unwrap()); // 5MB per stream
        transport_config.receive_window((50 * 1024 * 1024u32).try_into().unwrap()); // 50MB total
        transport_config.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));

        Ok(server_config)
    }

    /// 开始接收直播流
    async fn start_live_stream_reception(&self, session_id: Uuid, stream_id: String) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get_mut(&session_id) {
            // 创建直播流输出目录
            let live_dir = std::path::PathBuf::from("recv_live_streams");
            if let Err(e) = tokio::fs::create_dir_all(&live_dir).await {
                warn!("Failed to create live streams directory: {}", e);
            }
            
            // 生成带时间戳的文件名
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let output_filename = format!("{}_{}.h264", stream_id, timestamp);
            let output_path = live_dir.join(&output_filename);
            
            info!("🎬 Starting live stream reception: {} -> {:?}", stream_id, output_path);
            
            // 创建直播流信息
            let live_stream_info = LiveStreamInfo {
                stream_id: stream_id.clone(),
                output_path: output_path.clone(),
                start_time: std::time::SystemTime::now(),
                frames_received: 0,
                bytes_received: 0,
                is_active: true,
            };
            
            session.live_streams.insert(stream_id.clone(), live_stream_info);
            
            // 如果没有当前文件，创建一个用于直播流
            if session.current_file.is_none() {
                session.current_file = Some(ReceivingFileInfo {
                    file_path: format!("live_stream_{}", stream_id),
                    output_path: output_path.clone(),
                    total_size: 0,
                    received_size: 0,
                    video_segments: Vec::new(),
                    audio_segments: Vec::new(),
                    start_time: std::time::SystemTime::now(),
                    reception_start_time: None,
                    last_segment_time: None,
                    total_segments_received: 0,
                    average_receive_time_ms: 0.0,
                    peak_receive_throughput_mbps: 0.0,
                    is_live_stream: true,
                    live_stream_id: Some(stream_id.clone()),
                });
            }
            
            info!("✅ Live stream reception started: {}", stream_id);
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { 
                reason: format!("Session {} not found", session_id) 
            })
        }
    }
    
    /// 停止接收直播流
    async fn stop_live_stream_reception(&self, session_id: Uuid, stream_id: String) -> Result<(), TransportError> {
        let mut sessions = self.sessions.lock().await;
        
        if let Some(session) = sessions.get_mut(&session_id) {
            if let Some(mut live_stream) = session.live_streams.remove(&stream_id) {
                live_stream.is_active = false;
                
                let duration = live_stream.start_time.elapsed().unwrap_or_default();
                let avg_fps = if duration.as_secs_f64() > 0.0 {
                    live_stream.frames_received as f64 / duration.as_secs_f64()
                } else {
                    0.0
                };
                let avg_bitrate = if duration.as_secs_f64() > 0.0 {
                    (live_stream.bytes_received as f64 * 8.0) / duration.as_secs_f64() / 1_000_000.0
                } else {
                    0.0
                };
                
                info!("🛑 Live stream reception stopped: {}", stream_id);
                info!("   Duration: {:.2}s", duration.as_secs_f64());
                info!("   Frames: {}", live_stream.frames_received);
                info!("   Bytes: {}", live_stream.bytes_received);
                info!("   Avg FPS: {:.1}", avg_fps);
                info!("   Avg Bitrate: {:.1} Mbps", avg_bitrate);
                info!("   Output file: {:?}", live_stream.output_path);
                
                // 生成直播流报告
                self.generate_live_stream_report(&live_stream, duration).await?;
                
                // 如果当前文件是这个直播流，清除它
                if let Some(ref current_file) = session.current_file {
                    if current_file.is_live_stream && 
                       current_file.live_stream_id.as_ref() == Some(&stream_id) {
                        session.current_file = None;
                    }
                }
            }
            
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed { 
                reason: format!("Session {} not found", session_id) 
            })
        }
    }
    
    /// 生成直播流报告
    async fn generate_live_stream_report(&self, live_stream: &LiveStreamInfo, duration: std::time::Duration) -> Result<(), TransportError> {
        let report_path = live_stream.output_path.with_extension("report");
        let mp4_path = live_stream.output_path.with_extension("mp4");
        
        let avg_fps = if duration.as_secs_f64() > 0.0 {
            live_stream.frames_received as f64 / duration.as_secs_f64()
        } else {
            0.0
        };
        
        let avg_bitrate = if duration.as_secs_f64() > 0.0 {
            (live_stream.bytes_received as f64 * 8.0) / duration.as_secs_f64() / 1_000_000.0
        } else {
            0.0
        };
        
        let report_content = format!(
            "=== 直播流接收完成报告 ===\n\
             流ID: {}\n\
             开始时间: {:?}\n\
             持续时间: {:.2}秒\n\
             接收帧数: {}\n\
             接收字节数: {} bytes ({:.2} MB)\n\
             平均帧率: {:.1} fps\n\
             平均码率: {:.1} Mbps\n\
             \n\
             输出文件:\n\
             - H.264原始流: {:?}\n\
             - 转换后MP4: {:?} (需手动转换)\n\
             - 统计报告: {:?}\n\
             \n\
             转换命令:\n\
             ffmpeg -i {:?} -c copy {:?}\n\
             \n\
             播放命令:\n\
             ffplay {:?}\n\
             vlc {:?}\n\
             \n\
             说明:\n\
             - .h264 文件为原始H.264流，包含时间戳叠加\n\
             - 可使用上述命令转换为MP4格式播放\n\
             - 时间戳叠加可用于延迟测试\n\
             ===========================\n",
            live_stream.stream_id,
            live_stream.start_time,
            duration.as_secs_f64(),
            live_stream.frames_received,
            live_stream.bytes_received,
            live_stream.bytes_received as f64 / 1024.0 / 1024.0,
            avg_fps,
            avg_bitrate,
            live_stream.output_path,
            mp4_path,
            report_path,
            live_stream.output_path,
            mp4_path,
            live_stream.output_path,
            mp4_path
        );
        
        tokio::fs::write(&report_path, report_content).await
            .map_err(|e| TransportError::NetworkError { message: e.to_string() })?;
        
        info!("📊 Live stream report generated: {:?}", report_path);
        Ok(())
    }
}

/// 分片数据结构
#[derive(Debug, Clone)]
struct SegmentData {
    id: Uuid,
    timestamp: f64,
    duration: f64,
    data: Vec<u8>,
}

/// 文件请求载荷
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileRequestPayload {
    pub file_path: String,
    pub priority: u8,
    pub seek_position: Option<f64>,
    pub playback_rate: f64,
}

/// 播放控制命令
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PlaybackCommand {
    Seek { position: f64 },
    SetRate { rate: f64 },
    Pause,
    Resume,
    Stop,
    StartLive { 
        stream_id: String,
        quality: LiveStreamQuality,
        timestamp_overlay: bool,
    },
    StopLive { 
        stream_id: String,
    },
}

/// 直播流质量设置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiveStreamQuality {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe_interval: u32, // GOP size
}

/// 文件列表查询载荷
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileListQuery {
    pub filter: Option<String>, // 可选的文件过滤器
}

/// 文件列表响应载荷
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileListResponse {
    pub files: Vec<ClientFileInfo>,
}

/// 客户端文件信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClientFileInfo {
    pub file_path: String,
    pub file_size: u64,
    pub duration: Option<f64>,
    pub format: String,
    pub available: bool,
}

/// 会话统计信息
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub session_id: Uuid,
    pub status: SessionStatus,
    pub total_segments: usize,
    pub video_segments: usize,
    pub audio_segments: usize,
    pub requested_files: Vec<String>,
    pub current_file: Option<String>,
    pub received_size: u64,
    pub output_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[tokio::test]
    async fn test_server_creation() {
        let server = MockPlatformServer::new();
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_file_registration() {
        let server = MockPlatformServer::new().unwrap();
        
        let file_info = FileInfo {
            file_path: "test.mp4".to_string(),
            file_size: 1024 * 1024,
            duration: 60.0,
            available: true,
            priority: 1,
        };

        server.register_file("test.mp4".to_string(), file_info).await;
        
        let registry = server.file_registry.lock().await;
        assert!(registry.contains_key("test.mp4"));
    }
}