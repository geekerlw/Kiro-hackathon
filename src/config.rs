use std::path::{Path, PathBuf};
use std::time::Duration;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use crate::types::{QUICOptions, SegmentOptions, SegmentMode, ProtocolVersion};
use crate::errors::UploadManagerError;

/// Application configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Server connection settings
    pub server: ServerConfig,
    
    /// QUIC transport settings
    pub transport: TransportConfig,
    
    /// Video processing settings
    pub video: VideoConfig,
    
    /// Performance monitoring settings
    pub monitoring: MonitoringConfig,
    
    /// Error handling and recovery settings
    pub error_handling: ErrorHandlingConfig,
    
    /// Logging configuration
    pub logging: LoggingConfig,
    
    /// Resource limits and constraints
    pub resources: ResourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server address to connect to
    pub address: String,
    
    /// Server port
    pub port: u16,
    
    /// Connection timeout in seconds
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub connection_timeout: Duration,
    
    /// Protocol version to use
    pub protocol_version: ProtocolVersion,
    
    /// Enable TLS certificate verification
    pub verify_certificates: bool,
    
    /// Custom certificate path (optional)
    pub certificate_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Maximum concurrent streams
    pub max_concurrent_streams: u32,
    
    /// Initial maximum data per connection
    pub initial_max_data: u64,
    
    /// Initial maximum data per stream
    pub initial_max_stream_data: u64,
    
    /// Connection idle timeout
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub idle_timeout: Duration,
    
    /// Keep-alive interval
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub keep_alive_interval: Duration,
    
    /// Maximum packet size
    pub max_packet_size: usize,
    
    /// Enable adaptive congestion control
    pub adaptive_congestion_control: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    /// Default segmentation mode
    pub segment_mode: SegmentMode,
    
    /// Target segment size in bytes
    pub target_segment_size: Option<usize>,
    
    /// Maximum frames per segment
    pub max_frames_per_segment: Option<usize>,
    
    /// Enable audio/video separation
    pub enable_av_separation: bool,
    
    /// Supported video formats
    pub supported_formats: Vec<String>,
    
    /// Maximum video file size in bytes
    pub max_file_size: u64,
    
    /// Default frame rate for processing
    pub default_frame_rate: f64,
    
    /// Enable quality preservation
    pub preserve_quality: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable performance monitoring
    pub enabled: bool,
    
    /// Statistics collection interval
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub collection_interval: Duration,
    
    /// Maximum number of samples to keep
    pub max_samples: usize,
    
    /// Enable anomaly detection
    pub enable_anomaly_detection: bool,
    
    /// Anomaly detection thresholds
    pub anomaly_thresholds: AnomalyThresholds,
    
    /// Export formats to support
    pub export_formats: Vec<String>,
    
    /// Auto-export reports
    pub auto_export: bool,
    
    /// Export directory
    pub export_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyThresholds {
    /// High latency threshold in milliseconds
    pub high_latency_ms: u64,
    
    /// Low bandwidth threshold in bps
    pub low_bandwidth_bps: u64,
    
    /// High packet loss rate (0.0 to 1.0)
    pub high_packet_loss_rate: f64,
    
    /// Connection drop threshold (drops per minute)
    pub connection_drop_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorHandlingConfig {
    /// Enable automatic error recovery
    pub enable_recovery: bool,
    
    /// Maximum retry attempts
    pub max_retry_attempts: u32,
    
    /// Base retry delay in milliseconds
    pub base_retry_delay_ms: u64,
    
    /// Maximum retry delay in milliseconds
    pub max_retry_delay_ms: u64,
    
    /// Use exponential backoff
    pub exponential_backoff: bool,
    
    /// Enable circuit breaker pattern
    pub enable_circuit_breaker: bool,
    
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: u32,
    
    /// Circuit breaker timeout
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub circuit_breaker_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    pub level: String,
    
    /// Enable file logging
    pub enable_file_logging: bool,
    
    /// Log file path
    pub log_file_path: Option<PathBuf>,
    
    /// Maximum log file size in bytes
    pub max_log_file_size: u64,
    
    /// Number of log files to keep
    pub log_file_count: u32,
    
    /// Enable console logging
    pub enable_console_logging: bool,
    
    /// Log format (json, text)
    pub log_format: String,
    
    /// Enable structured logging
    pub structured_logging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    /// Maximum memory usage in MB
    pub max_memory_mb: u64,
    
    /// Maximum CPU usage percentage
    pub max_cpu_percentage: f64,
    
    /// Maximum disk space usage in MB
    pub max_disk_space_mb: u64,
    
    /// Buffer size limits
    pub buffer_limits: BufferLimits,
    
    /// Enable resource monitoring
    pub enable_monitoring: bool,
    
    /// Resource check interval
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub check_interval: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferLimits {
    /// Maximum video buffer size in bytes
    pub max_video_buffer_size: usize,
    
    /// Maximum audio buffer size in bytes
    pub max_audio_buffer_size: usize,
    
    /// Maximum transmission queue size
    pub max_transmission_queue_size: usize,
    
    /// Buffer health check interval
    #[serde(serialize_with = "crate::serde_helpers::serialize_duration")]
    #[serde(deserialize_with = "crate::serde_helpers::deserialize_duration")]
    pub health_check_interval: Duration,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            transport: TransportConfig::default(),
            video: VideoConfig::default(),
            monitoring: MonitoringConfig::default(),
            error_handling: ErrorHandlingConfig::default(),
            logging: LoggingConfig::default(),
            resources: ResourceConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 4433,
            connection_timeout: Duration::from_secs(30),
            protocol_version: ProtocolVersion::CURRENT,
            verify_certificates: true,
            certificate_path: None,
        }
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_concurrent_streams: 100,
            initial_max_data: 50 * 1024 * 1024, // 50MB - increased for large file transfers
            initial_max_stream_data: 5 * 1024 * 1024, // 5MB - increased to support large segments
            idle_timeout: Duration::from_secs(30),
            keep_alive_interval: Duration::from_secs(10),
            max_packet_size: 1350, // Standard MTU minus headers
            adaptive_congestion_control: true,
        }
    }
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            segment_mode: SegmentMode::Frame,
            target_segment_size: Some(64 * 1024), // 64KB
            max_frames_per_segment: Some(30),
            enable_av_separation: true,
            supported_formats: vec!["mp4".to_string(), "h264".to_string()],
            max_file_size: 10 * 1024 * 1024 * 1024, // 10GB
            default_frame_rate: 30.0,
            preserve_quality: true,
        }
    }
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            collection_interval: Duration::from_millis(100),
            max_samples: 1000,
            enable_anomaly_detection: true,
            anomaly_thresholds: AnomalyThresholds::default(),
            export_formats: vec!["json".to_string(), "csv".to_string()],
            auto_export: false,
            export_directory: None,
        }
    }
}

impl Default for AnomalyThresholds {
    fn default() -> Self {
        Self {
            high_latency_ms: 1000, // 1 second
            low_bandwidth_bps: 1_000_000, // 1 Mbps
            high_packet_loss_rate: 0.05, // 5%
            connection_drop_threshold: 5, // 5 drops per minute
        }
    }
}

impl Default for ErrorHandlingConfig {
    fn default() -> Self {
        Self {
            enable_recovery: true,
            max_retry_attempts: 5,
            base_retry_delay_ms: 100,
            max_retry_delay_ms: 30000, // 30 seconds
            exponential_backoff: true,
            enable_circuit_breaker: true,
            circuit_breaker_threshold: 10,
            circuit_breaker_timeout: Duration::from_secs(60),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            enable_file_logging: true,
            log_file_path: Some(PathBuf::from("video_uploader.log")),
            max_log_file_size: 10 * 1024 * 1024, // 10MB
            log_file_count: 5,
            enable_console_logging: true,
            log_format: "text".to_string(),
            structured_logging: false,
        }
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 1024, // 1GB
            max_cpu_percentage: 80.0,
            max_disk_space_mb: 5 * 1024, // 5GB
            buffer_limits: BufferLimits::default(),
            enable_monitoring: true,
            check_interval: Duration::from_secs(5),
        }
    }
}

impl Default for BufferLimits {
    fn default() -> Self {
        Self {
            max_video_buffer_size: 50 * 1024 * 1024, // 50MB
            max_audio_buffer_size: 10 * 1024 * 1024, // 10MB
            max_transmission_queue_size: 100,
            health_check_interval: Duration::from_secs(1),
        }
    }
}

/// Configuration manager for loading, saving, and validating configurations
pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            config: AppConfig::default(),
        }
    }

    /// Load configuration from file
    pub async fn load(&mut self) -> Result<(), UploadManagerError> {
        if !self.config_path.exists() {
            // Create default configuration file
            self.save().await?;
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&self.config_path).await
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Failed to read config file: {}", e) 
            })?;

        self.config = toml::from_str(&content)
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Failed to parse config file: {}", e) 
            })?;

        self.validate()?;
        Ok(())
    }

    /// Save configuration to file
    pub async fn save(&self) -> Result<(), UploadManagerError> {
        let content = toml::to_string_pretty(&self.config)
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Failed to serialize config: {}", e) 
            })?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = self.config_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| UploadManagerError::ConfigurationError { 
                    message: format!("Failed to create config directory: {}", e) 
                })?;
        }

        tokio::fs::write(&self.config_path, content).await
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Failed to write config file: {}", e) 
            })?;

        Ok(())
    }

    /// Get the current configuration
    pub fn get_config(&self) -> &AppConfig {
        &self.config
    }

    /// Get a mutable reference to the configuration
    pub fn get_config_mut(&mut self) -> &mut AppConfig {
        &mut self.config
    }

    /// Update configuration
    pub fn update_config(&mut self, config: AppConfig) -> Result<(), UploadManagerError> {
        self.config = config;
        self.validate()?;
        Ok(())
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), UploadManagerError> {
        // Validate server configuration
        if self.config.server.address.is_empty() {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Server address cannot be empty".to_string() 
            });
        }

        if self.config.server.port == 0 {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Server port must be greater than 0".to_string() 
            });
        }

        // Validate transport configuration
        if self.config.transport.max_concurrent_streams == 0 {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Max concurrent streams must be greater than 0".to_string() 
            });
        }

        // Validate video configuration
        if self.config.video.supported_formats.is_empty() {
            return Err(UploadManagerError::ConfigurationError { 
                message: "At least one video format must be supported".to_string() 
            });
        }

        if self.config.video.default_frame_rate <= 0.0 {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Default frame rate must be greater than 0".to_string() 
            });
        }

        // Validate resource limits
        if self.config.resources.max_memory_mb == 0 {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Max memory limit must be greater than 0".to_string() 
            });
        }

        if self.config.resources.max_cpu_percentage <= 0.0 || self.config.resources.max_cpu_percentage > 100.0 {
            return Err(UploadManagerError::ConfigurationError { 
                message: "Max CPU percentage must be between 0 and 100".to_string() 
            });
        }

        Ok(())
    }

    /// Convert to UploadConfig for UploadManager
    pub fn to_upload_config(&self) -> Result<crate::upload_manager::UploadConfig, UploadManagerError> {
        let server_addr: SocketAddr = format!("{}:{}", self.config.server.address, self.config.server.port)
            .parse()
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Invalid server address: {}", e) 
            })?;

        Ok(crate::upload_manager::UploadConfig {
            server_address: server_addr,
            quic_options: QUICOptions {
                max_concurrent_streams: self.config.transport.max_concurrent_streams,
                initial_max_data: self.config.transport.initial_max_data,
                initial_max_stream_data: self.config.transport.initial_max_stream_data,
                idle_timeout: self.config.transport.idle_timeout,
            },
            segment_options: SegmentOptions {
                segment_mode: self.config.video.segment_mode.clone(),
                target_size: self.config.video.target_segment_size,
                max_frames_per_segment: self.config.video.max_frames_per_segment,
            },
            max_concurrent_uploads: self.config.transport.max_concurrent_streams as usize,
            enable_performance_monitoring: self.config.monitoring.enabled,
            enable_error_recovery: self.config.error_handling.enable_recovery,
            protocol_version: self.config.server.protocol_version.clone(),
        })
    }
}

/// Environment-based configuration override
pub struct EnvConfigOverride;

impl EnvConfigOverride {
    /// Apply environment variable overrides to configuration
    pub fn apply_overrides(config: &mut AppConfig) {
        use std::env;

        // Server overrides
        if let Ok(addr) = env::var("VIDEO_UPLOADER_SERVER_ADDRESS") {
            config.server.address = addr;
        }
        if let Ok(port) = env::var("VIDEO_UPLOADER_SERVER_PORT") {
            if let Ok(port) = port.parse::<u16>() {
                config.server.port = port;
            }
        }

        // Transport overrides
        if let Ok(streams) = env::var("VIDEO_UPLOADER_MAX_STREAMS") {
            if let Ok(streams) = streams.parse::<u32>() {
                config.transport.max_concurrent_streams = streams;
            }
        }

        // Monitoring overrides
        if let Ok(enabled) = env::var("VIDEO_UPLOADER_MONITORING_ENABLED") {
            config.monitoring.enabled = enabled.to_lowercase() == "true";
        }

        // Logging overrides
        if let Ok(level) = env::var("VIDEO_UPLOADER_LOG_LEVEL") {
            config.logging.level = level;
        }
    }
}