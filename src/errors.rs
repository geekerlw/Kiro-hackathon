use thiserror::Error;
use std::io;

#[derive(Error, Debug)]
pub enum FileError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    
    #[error("Unsupported file format: {format}")]
    UnsupportedFormat { format: String },
    
    #[error("File is corrupted or unreadable")]
    CorruptedFile,
    
    #[error("Insufficient permissions to access file")]
    PermissionDenied,
    
    #[error("File not found: {path}")]
    FileNotFound { path: String },
    
    #[error("Invalid file metadata")]
    InvalidMetadata,
    
    #[error("IO error: {message}")]
    IoError { message: String },
    
    #[error("Invalid seek position")]
    InvalidSeekPosition,
    
    #[error("Seek operation failed")]
    SeekFailed,
    
    #[error("Seek position beyond end of file")]
    SeekBeyondEnd,
    
    #[error("FFmpeg error: {0}")]
    FFmpegError(#[from] FFmpegError),
    
    #[error("Timeline error: {0}")]
    TimelineError(#[from] TimelineError),
    
    #[error("No video stream found in file")]
    NoVideoStream,
}

#[derive(Error, Debug)]
pub enum SegmentError {
    #[error("Failed to find key frames in video data")]
    KeyFrameNotFound,
    
    #[error("Invalid segment parameters")]
    InvalidParameters,
    
    #[error("Encoding error: {message}")]
    EncodingError { message: String },
    
    #[error("Buffer overflow during segmentation")]
    BufferOverflow,
    
    #[error("Segmentation interrupted")]
    Interrupted,
}

#[derive(Error, Debug)]
pub enum SeparationError {
    #[error("No audio track found in video file")]
    NoAudioTrack,
    
    #[error("Failed to separate audio and video streams")]
    SeparationFailed,
    
    #[error("Stream synchronization error")]
    SyncError,
    
    #[error("Invalid stream format")]
    InvalidStreamFormat,
}

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Failed to establish QUIC connection: {reason}")]
    ConnectionFailed { reason: String },
    
    #[error("Network timeout after {duration:?}")]
    Timeout { duration: std::time::Duration },
    
    #[error("Connection lost: {reason}")]
    ConnectionLost { reason: String },
    
    #[error("Invalid server response: {details}")]
    InvalidResponse { details: String },
    
    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },
    
    #[error("Server error: {code} - {message}")]
    ServerError { code: u16, message: String },
    
    #[error("Protocol version mismatch: client {client_version}, server {server_version}")]
    ProtocolMismatch { client_version: String, server_version: String },
    
    #[error("Serialization error: {message}")]
    SerializationError { message: String },
    
    #[error("Network error: {message}")]
    NetworkError { message: String },
    
    #[error("Invalid data: {message}")]
    InvalidData { message: String },
    
    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },
    
    #[error("Stream creation failed: {reason}")]
    StreamCreationFailed { reason: String },
    
    #[error("Network unreachable: {address}")]
    NetworkUnreachable { address: String },
    
    #[error("DNS resolution failed: {hostname}")]
    DnsResolutionFailed { hostname: String },
    
    #[error("TLS handshake failed: {reason}")]
    TlsHandshakeFailed { reason: String },
    
    #[error("Certificate verification failed: {reason}")]
    CertificateVerificationFailed { reason: String },
    
    #[error("Bandwidth insufficient: required {required_bps} bps, available {available_bps} bps")]
    InsufficientBandwidth { required_bps: u64, available_bps: u64 },
    
    #[error("Rate limit exceeded: {retry_after:?}")]
    RateLimitExceeded { retry_after: Option<std::time::Duration> },
    
    #[error("Protocol error: {error_code} - {description}")]
    ProtocolError { error_code: u16, description: String },
    
    #[error("Deserialization error: {message}")]
    DeserializationError { message: String },
}

#[derive(Error, Debug)]
pub enum PlaybackError {
    #[error("Invalid seek position: {position}")]
    InvalidSeekPosition { position: f64 },
    
    #[error("Invalid playback rate: {rate}")]
    InvalidPlaybackRate { rate: f64 },
    
    #[error("Seek operation failed")]
    SeekFailed,
    
    #[error("Buffer management error")]
    BufferError,
    
    #[error("Synchronization lost")]
    SyncLost,
    
    #[error("Keyframe not found at timestamp: {timestamp}")]
    KeyframeNotFound { timestamp: f64 },
    
    #[error("Invalid keyframe index: {reason}")]
    InvalidKeyframeIndex { reason: String },
}

#[derive(Error, Debug)]
pub enum MonitorError {
    #[error("Failed to start monitoring")]
    StartupFailed,
    
    #[error("Data collection error")]
    DataCollectionError,
    
    #[error("Report generation failed")]
    ReportGenerationFailed,
    
    #[error("Export error: {format:?}")]
    ExportError { format: crate::types::ExportFormat },
}

#[derive(Error, Debug)]
pub enum ExportError {
    #[error("Unsupported export format")]
    UnsupportedFormat,
    
    #[error("Data serialization failed")]
    SerializationFailed,
    
    #[error("File write error: {0}")]
    WriteError(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("Processing stage failed: {stage}")]
    StageFailed { stage: String },
    
    #[error("Pipeline error: {message}")]
    PipelineError { message: String },
    
    #[error("Resource exhausted")]
    ResourceExhausted,
}

// Network error handling specific errors
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection retry exhausted: {attempts} attempts failed")]
    RetryExhausted { attempts: u32 },
    
    #[error("Server response parsing failed: {reason}")]
    ResponseParsingFailed { reason: String },
    
    #[error("Protocol negotiation failed: {reason}")]
    ProtocolNegotiationFailed { reason: String },
    
    #[error("Network condition degraded: {condition}")]
    NetworkDegraded { condition: String },
    
    #[error("Connection recovery failed: {reason}")]
    RecoveryFailed { reason: String },
}

// System resource monitoring errors
#[derive(Error, Debug)]
pub enum ResourceError {
    #[error("Insufficient storage space: {available} bytes available, {required} bytes required")]
    InsufficientStorage { available: u64, required: u64 },
    
    #[error("File integrity check failed: {file_path}")]
    FileIntegrityFailed { file_path: String },
    
    #[error("File modification detected during transmission: {file_path}")]
    FileModificationDetected { file_path: String },
    
    #[error("Memory usage exceeded limit: {current} MB used, {limit} MB limit")]
    MemoryLimitExceeded { current: u64, limit: u64 },
    
    #[error("CPU usage too high: {current}% used, {threshold}% threshold")]
    CpuUsageHigh { current: f64, threshold: f64 },
    
    #[error("System resource monitoring failed: {reason}")]
    MonitoringFailed { reason: String },
}

// Upload manager specific errors
#[derive(Error, Debug)]
pub enum UploadManagerError {
    #[error("File error: {0}")]
    FileError(#[from] FileError),
    
    #[error("Segmentation error: {0}")]
    SegmentError(#[from] SegmentError),
    
    #[error("Separation error: {0}")]
    SeparationError(#[from] SeparationError),
    
    #[error("Transport error: {0}")]
    TransportError(#[from] TransportError),
    
    #[error("Playback error: {0}")]
    PlaybackError(#[from] PlaybackError),
    
    #[error("Export error: {0}")]
    ExportError(#[from] ExportError),
    
    #[error("Network error: {0}")]
    NetworkError(#[from] NetworkError),
    
    #[error("Resource error: {0}")]
    ResourceError(#[from] ResourceError),
    
    #[error("File not found: {0}")]
    FileNotFound(String),
    
    #[error("Session already active")]
    SessionAlreadyActive,
    
    #[error("No active session")]
    NoActiveSession,
    
    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },
    
    #[error("Component initialization failed: {component}")]
    ComponentInitializationFailed { component: String },
    
    #[error("Performance monitoring is disabled")]
    MonitoringDisabled,
    
    #[error("Session management error: {message}")]
    SessionManagementError { message: String },
    
    #[error("Event handling error: {message}")]
    EventHandlingError { message: String },
}

// Main application error type
#[derive(Error, Debug)]
pub enum VideoStreamingError {
    #[error("File error: {0}")]
    File(#[from] FileError),
    
    #[error("Segmentation error: {0}")]
    Segment(#[from] SegmentError),
    
    #[error("Stream separation error: {0}")]
    Separation(#[from] SeparationError),
    
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    
    #[error("Playback error: {0}")]
    Playback(#[from] PlaybackError),
    
    #[error("Monitoring error: {0}")]
    Monitor(#[from] MonitorError),
    
    #[error("Export error: {0}")]
    Export(#[from] ExportError),
    
    #[error("Processing error: {0}")]
    Processing(#[from] ProcessingError),
    
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),
    
    #[error("Resource error: {0}")]
    Resource(#[from] ResourceError),
    
    #[error("Upload manager error: {0}")]
    UploadManager(#[from] UploadManagerError),
    
    #[error("Configuration error: {message}")]
    Configuration { message: String },
    
    #[error("System resource error: {message}")]
    SystemResource { message: String },
    
    #[error("FFmpeg error: {0}")]
    FFmpeg(#[from] FFmpegError),
    
    #[error("Timeline error: {0}")]
    Timeline(#[from] TimelineError),
}

// FFmpeg command line integration errors
#[derive(Error, Debug)]
pub enum FFmpegError {
    #[error("FFmpeg not available on this system")]
    NotAvailable,
    
    #[error("FFmpeg command failed: {0}")]
    CommandFailed(String),
    
    #[error("Failed to parse FFmpeg output: {0}")]
    ParseError(String),
    
    #[error("File operation error: {0}")]
    FileError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    
    #[error("Timeline file corrupted: {0}")]
    TimelineCorrupted(String),
    
    #[error("Unsupported video format: {0}")]
    UnsupportedFormat(String),
    
    #[error("FFmpeg version incompatible: {version}")]
    IncompatibleVersion { version: String },
    
    #[error("Parsing cancelled by user")]
    Cancelled,
    
    #[error("Parsing timeout after {duration:?}")]
    Timeout { duration: std::time::Duration },
}

// Timeline file management errors
#[derive(Error, Debug)]
pub enum TimelineError {
    #[error("Timeline file not found: {0}")]
    NotFound(String),
    
    #[error("Timeline file corrupted: {0}")]
    Corrupted(String),
    
    #[error("Timeline file outdated: video modified after timeline generation")]
    Outdated,
    
    #[error("Timeline cache full: {current_size} MB used, {limit} MB limit")]
    CacheFull { current_size: u64, limit: u64 },
    
    #[error("Timeline validation failed: {reason}")]
    ValidationFailed { reason: String },
    
    #[error("FFmpeg error: {0}")]
    FFmpeg(#[from] FFmpegError),
    
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    
    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),
}