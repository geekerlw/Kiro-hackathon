use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use tokio_stream::Stream;

// Core video file information
#[derive(Debug, Clone)]
pub struct VideoFileInfo {
    pub duration: f64,
    pub resolution: Resolution,
    pub codec: String,
    pub frame_rate: f64,
    pub bit_rate: u64,
    pub has_audio: bool,
}

// Keyframe index structures for precise seek operations
#[derive(Debug, Clone)]
pub struct KeyframeIndex {
    pub entries: Vec<KeyframeEntry>,
    pub total_duration: f64,
    pub index_precision: f64, // 索引精度，支持亚秒级
    pub memory_optimized: bool,
    pub optimization_strategy: IndexOptimizationStrategy,
    pub memory_usage: usize,
}

#[derive(Debug, Clone)]
pub struct KeyframeEntry {
    pub timestamp: f64,        // 时间戳（秒）
    pub file_offset: u64,      // 文件偏移位置
    pub frame_size: u32,       // 关键帧大小
    pub gop_size: u32,         // GOP大小
    pub frame_type: FrameType, // 帧类型
}

#[derive(Debug, Clone)]
pub enum FrameType {
    I,  // Intra frame (keyframe)
    P,  // Predicted frame
    B,  // Bidirectional frame
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndexOptimizationStrategy {
    Full,           // 完整索引，所有关键帧
    Sparse,         // 稀疏索引，定期采样
    Adaptive,       // 自适应，根据内存动态调整
    Hierarchical,   // 分层索引，多级精度
}

// Keyframe index manager for memory optimization
#[derive(Debug, Clone)]
pub struct KeyframeIndexManager {
    pub indices: HashMap<PathBuf, KeyframeIndex>,
    pub memory_limit: usize,
    pub current_memory_usage: usize,
    pub optimization_strategy: IndexOptimizationStrategy,
}

// Seek operation result
#[derive(Debug, Clone)]
pub struct SeekResult {
    pub requested_time: f64,
    pub actual_time: f64,
    pub keyframe_offset: u64,
    pub precision_achieved: f64,
    pub keyframe_used: KeyframeEntry,
    pub execution_time: Duration,
}

// Seek operation record
#[derive(Debug, Clone)]
pub struct SeekOperation {
    pub id: Uuid,
    pub requested_time: f64,
    pub actual_time: f64,
    pub target_offset: u64,
    pub keyframe_used: KeyframeEntry,
    pub seek_accuracy: f64,
    pub execution_time: Duration,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: String,
}

// Video codec information
#[derive(Debug, Clone)]
pub struct VideoCodec {
    pub name: String,
    pub profile: String,
    pub level: String,
    pub color_space: String,
    pub bit_depth: u8,
}

// Audio track information
#[derive(Debug, Clone)]
pub struct AudioTrackInfo {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_rate: u64,
    pub language: Option<String>,
}

// Video metadata
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub duration: f64,
    pub resolution: Resolution,
    pub codec: VideoCodec,
    pub frame_rate: f64,
    pub bit_rate: u64,
    pub audio_tracks: Vec<AudioTrackInfo>,
    pub created_at: SystemTime,
    pub checksum: String,
}

// Segmentation options
#[derive(Debug, Clone)]
pub struct SegmentOptions {
    pub segment_mode: SegmentMode,
    pub target_size: Option<usize>,
    pub max_frames_per_segment: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SegmentMode {
    Frame,
    Gop,
    Time,
}

// Segment metadata
#[derive(Debug, Clone)]
pub struct SegmentMetadata {
    pub frame_indices: Vec<usize>,
    pub key_frame_positions: Vec<usize>,
    pub encoding_params: HashMap<String, String>,
}

// Video segment
#[derive(Debug, Clone)]
pub struct VideoSegment {
    pub id: Uuid,
    pub data: Vec<u8>,
    pub timestamp: f64,
    pub duration: f64,
    pub frame_count: usize,
    pub is_key_frame: bool,
    pub metadata: SegmentMetadata,
}

// Audio segment
#[derive(Debug, Clone)]
pub struct AudioSegment {
    pub id: Uuid,
    pub data: Vec<u8>,
    pub timestamp: f64,
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

// Synchronization information
#[derive(Debug, Clone)]
pub struct SyncInfo {
    pub video_timestamp: f64,
    pub audio_timestamp: f64,
    pub offset: f64,
}

// Separated streams
pub struct SeparatedStreams {
    pub video_stream: Box<dyn Stream<Item = VideoSegment> + Unpin + Send>,
    pub audio_stream: Box<dyn Stream<Item = AudioSegment> + Unpin + Send>,
}

impl std::fmt::Debug for SeparatedStreams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeparatedStreams")
            .field("video_stream", &"<video stream>")
            .field("audio_stream", &"<audio stream>")
            .finish()
    }
}

// QUIC connection options
#[derive(Debug, Clone)]
pub struct QUICOptions {
    pub max_concurrent_streams: u32,
    pub initial_max_data: u64,
    pub initial_max_stream_data: u64,
    pub idle_timeout: Duration,
}

// Stream types
#[derive(Debug, Clone, Copy)]
pub enum StreamType {
    Video,
    Audio,
}

// Stream configuration for different types
#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub stream_type: StreamType,
    pub priority: u8,
    pub max_data_rate: u64,
    pub buffer_size: usize,
    pub congestion_control: CongestionControlType,
}

#[derive(Debug, Clone)]
pub enum CongestionControlType {
    LowLatency,    // For audio streams
    HighThroughput, // For video streams
    Adaptive,      // Adjusts based on network conditions
}

// Network condition monitoring
#[derive(Debug, Clone)]
pub struct NetworkConditions {
    pub bandwidth_estimate: u64,
    pub rtt: Duration,
    pub packet_loss_rate: f64,
    pub jitter: Duration,
    pub congestion_level: CongestionLevel,
}

#[derive(Debug, Clone)]
pub enum CongestionLevel {
    Low,
    Medium,
    High,
    Critical,
}

// Protocol message types for language-agnostic communication
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProtocolMessage {
    pub message_type: MessageType,
    pub payload: Vec<u8>,
    pub sequence_number: u64,
    #[serde(with = "crate::serde_helpers::systemtime")]
    pub timestamp: SystemTime,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MessageType {
    SessionStart = 0x01,
    SessionEnd = 0x02,
    SeekRequest = 0x03,
    RateChange = 0x04,
    PauseRequest = 0x05,
    ResumeRequest = 0x06,
    ErrorReport = 0x07,
    StatsRequest = 0x08,
    StatusResponse = 0x09,
    VersionNegotiation = 0x0A,
    FileRequest = 0x0B,
    PlaybackControl = 0x0C,
    FileListQuery = 0x0D,
    FileListResponse = 0x0E,
    LiveStreamControl = 0x0F,
}

// Protocol version information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProtocolVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl ProtocolVersion {
    pub const CURRENT: ProtocolVersion = ProtocolVersion {
        major: 1,
        minor: 0,
        patch: 0,
    };

    pub fn is_compatible(&self, other: &ProtocolVersion) -> bool {
        self.major == other.major && self.minor <= other.minor
    }
}

impl RecoveryState {
    pub fn new() -> Self {
        Self {
            last_successful_segment: None,
            retry_count: 0,
            max_retries: 5,
            backoff_duration: Duration::from_millis(0),
            recovery_strategy: RecoveryStrategy::ExponentialBackoff,
        }
    }

    pub fn with_strategy(strategy: RecoveryStrategy) -> Self {
        Self {
            last_successful_segment: None,
            retry_count: 0,
            max_retries: 5,
            backoff_duration: Duration::from_millis(0),
            recovery_strategy: strategy,
        }
    }
}

// Connection recovery state
#[derive(Debug, Clone)]
pub struct RecoveryState {
    pub last_successful_segment: Option<uuid::Uuid>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub backoff_duration: Duration,
    pub recovery_strategy: RecoveryStrategy,
}

#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    Immediate,
    ExponentialBackoff,
    LinearBackoff,
    AdaptiveBackoff,
}

// Status response codes
#[derive(Debug, Clone)]
pub enum StatusCode {
    Success = 200,
    BadRequest = 400,
    Unauthorized = 401,
    NotFound = 404,
    InternalError = 500,
    ServiceUnavailable = 503,
    UnsupportedFormat = 1001,
    InsufficientBandwidth = 1002,
    StorageFull = 1003,
    AuthenticationFailed = 1004,
    SegmentCorrupted = 1005,
}

// Generic segment enum
#[derive(Debug, Clone)]
pub enum Segment {
    Video(VideoSegment),
    Audio(AudioSegment),
}

// Connection statistics
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub rtt: Duration,
    pub bandwidth: u64,
    pub packet_loss: f64,
    pub congestion_window: u64,
}

// Frame dropping strategy
#[derive(Debug, Clone)]
pub struct DropFrameStrategy {
    pub drop_b_frames: bool,
    pub drop_p_frames: bool,
    pub keep_key_frames_only: bool,
    pub adaptive_dropping: bool,
}

// Performance statistics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceStats {
    pub current_upload_speed: f64,
    pub average_upload_speed: f64,
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub current_latency: Duration,
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub average_latency: Duration,
    pub total_bytes_transmitted: u64,
    pub segments_transmitted: u64,
}

// Performance report
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceReport {
    pub session_id: Uuid,
    #[serde(serialize_with = "crate::serde_helpers::serialize_system_time")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_system_time")]
    pub start_time: SystemTime,
    #[serde(serialize_with = "crate::serde_helpers::serialize_system_time")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_system_time")]
    pub end_time: SystemTime,
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub total_duration: Duration,
    pub stats: PerformanceStats,
    pub peak_upload_speed: f64,
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration_vec")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration_vec")]
    pub latency_distribution: Vec<Duration>,
    pub anomalies: Vec<PerformanceAnomaly>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceAnomaly {
    #[serde(serialize_with = "crate::serde_helpers::serialize_system_time")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_system_time")]
    pub timestamp: SystemTime,
    pub anomaly_type: AnomalyType,
    pub description: String,
    pub severity: AnomalySeverity,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AnomalyType {
    HighLatency,
    LowBandwidth,
    PacketLoss,
    ConnectionDrop,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

// Export formats
#[derive(Debug, Clone)]
pub enum ExportFormat {
    Json,
    Csv,
}

// Transmission session
#[derive(Debug, Clone)]
pub struct TransmissionSession {
    pub id: Uuid,
    pub video_metadata: VideoMetadata,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub status: TransmissionStatus,
    pub progress: f64,
    pub current_position: f64,
    pub playback_rate: f64,
    pub total_segments: u64,
    pub transmitted_segments: u64,
    pub errors: Vec<TransmissionError>,
}

#[derive(Debug, Clone)]
pub enum TransmissionStatus {
    Preparing,
    Active,
    Paused,
    Completed,
    Failed,
}

// Transmission error
#[derive(Debug, Clone)]
pub struct TransmissionError {
    pub timestamp: SystemTime,
    pub error_type: ErrorType,
    pub message: String,
    pub segment_id: Option<Uuid>,
    pub retry_count: u32,
    pub resolved: bool,
}

#[derive(Debug, Clone)]
pub enum ErrorType {
    Network,
    Encoding,
    File,
    Server,
}

// Buffer management
#[derive(Debug)]
pub struct BufferManager {
    pub video_buffers: HashMap<Uuid, Vec<VideoSegment>>,
    pub audio_buffers: HashMap<Uuid, Vec<AudioSegment>>,
    pub max_buffer_size: usize,
    pub current_buffer_size: usize,
    pub buffer_health: BufferHealth,
}

#[derive(Debug, Clone)]
pub struct BufferHealth {
    pub video_buffer_level: f64,
    pub audio_buffer_level: f64,
    pub underrun_count: u32,
    pub overrun_count: u32,
    pub last_underrun: Option<SystemTime>,
}

// QUIC connection and stream types
#[derive(Clone)]
pub struct QUICConnection {
    pub id: Uuid,
    pub remote_address: std::net::SocketAddr,
    pub established_at: SystemTime,
    pub stats: ConnectionStats,
    pub inner: Arc<quinn::Connection>,
    pub config: QUICOptions,
}

impl std::fmt::Debug for QUICConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QUICConnection")
            .field("id", &self.id)
            .field("remote_address", &self.remote_address)
            .field("established_at", &self.established_at)
            .field("stats", &self.stats)
            .field("config", &self.config)
            .finish()
    }
}

pub struct QUICStream {
    pub id: u64,
    pub stream_type: StreamType,
    pub created_at: SystemTime,
    pub send_stream: Option<quinn::SendStream>,
    pub recv_stream: Option<quinn::RecvStream>,
}

impl std::fmt::Debug for QUICStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QUICStream")
            .field("id", &self.id)
            .field("stream_type", &self.stream_type)
            .field("created_at", &self.created_at)
            .field("has_send_stream", &self.send_stream.is_some())
            .field("has_recv_stream", &self.recv_stream.is_some())
            .finish()
    }
}