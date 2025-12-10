use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn, error, debug};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{ConfigManager, AppConfig, EnvConfigOverride};
use crate::upload_manager::{UploadManager, UploadConfig, EventHandler, ConsoleEventHandler};
use crate::file_reader::{FileStreamReader, DefaultFileStreamReader};
use crate::segmenter::{VideoSegmenter, DefaultVideoSegmenter};
use crate::separator::DefaultAudioVideoSeparator;
use crate::transport::{QUICTransport, DefaultQUICTransport};
use crate::controller::{PlaybackController, DefaultPlaybackController};
use crate::monitor::{PerformanceMonitor, DefaultPerformanceMonitor};
use crate::network_error_handler::DefaultNetworkErrorHandler;
use crate::resource_monitor::DefaultSystemResourceMonitor;
use crate::errors::UploadManagerError;

/// Application builder for dependency injection and initialization
pub struct AppBuilder {
    config_path: Option<PathBuf>,
    custom_config: Option<AppConfig>,
    custom_file_reader: Option<Arc<dyn FileStreamReader + Send + Sync>>,
    custom_segmenter: Option<Arc<dyn VideoSegmenter + Send + Sync>>,
    custom_separator: Option<Arc<DefaultAudioVideoSeparator>>,
    custom_transport: Option<Arc<dyn QUICTransport + Send + Sync>>,
    custom_controller: Option<Arc<Mutex<dyn PlaybackController + Send + Sync>>>,
    custom_monitor: Option<Arc<Mutex<dyn PerformanceMonitor + Send + Sync>>>,
    event_handlers: Vec<Arc<dyn EventHandler>>,
    enable_env_overrides: bool,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppBuilder {
    /// Create a new application builder
    pub fn new() -> Self {
        Self {
            config_path: None,
            custom_config: None,
            custom_file_reader: None,
            custom_segmenter: None,
            custom_separator: None,
            custom_transport: None,
            custom_controller: None,
            custom_monitor: None,
            event_handlers: Vec::new(),
            enable_env_overrides: true,
        }
    }

    /// Set configuration file path
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Set custom configuration
    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.custom_config = Some(config);
        self
    }

    /// Set custom file reader implementation
    pub fn with_file_reader(mut self, file_reader: Arc<dyn FileStreamReader + Send + Sync>) -> Self {
        self.custom_file_reader = Some(file_reader);
        self
    }

    /// Set custom video segmenter implementation
    pub fn with_segmenter(mut self, segmenter: Arc<dyn VideoSegmenter + Send + Sync>) -> Self {
        self.custom_segmenter = Some(segmenter);
        self
    }

    /// Set custom audio/video separator implementation
    pub fn with_separator(mut self, separator: Arc<DefaultAudioVideoSeparator>) -> Self {
        self.custom_separator = Some(separator);
        self
    }

    /// Set custom QUIC transport implementation
    pub fn with_transport(mut self, transport: Arc<dyn QUICTransport + Send + Sync>) -> Self {
        self.custom_transport = Some(transport);
        self
    }

    /// Set custom playback controller implementation
    pub fn with_controller(mut self, controller: Arc<Mutex<dyn PlaybackController + Send + Sync>>) -> Self {
        self.custom_controller = Some(controller);
        self
    }

    /// Set custom performance monitor implementation
    pub fn with_monitor(mut self, monitor: Arc<Mutex<dyn PerformanceMonitor + Send + Sync>>) -> Self {
        self.custom_monitor = Some(monitor);
        self
    }

    /// Add an event handler
    pub fn with_event_handler(mut self, handler: Arc<dyn EventHandler>) -> Self {
        self.event_handlers.push(handler);
        self
    }

    /// Enable or disable environment variable overrides
    pub fn with_env_overrides(mut self, enable: bool) -> Self {
        self.enable_env_overrides = enable;
        self
    }

    /// Build the application
    pub async fn build(self) -> Result<VideoStreamingApp, UploadManagerError> {
        // Initialize logging first
        self.init_logging().await?;

        info!("Initializing Video Streaming Uploader application");

        // Load configuration
        let config = self.load_configuration().await?;
        debug!("Configuration loaded successfully");

        // Create upload configuration
        let upload_config = self.create_upload_config(&config)?;

        // Create components with dependency injection
        let upload_manager = self.create_upload_manager(upload_config).await?;

        // Add event handlers
        for handler in &self.event_handlers {
            upload_manager.add_event_handler(handler.clone()).await;
        }

        // Add default console handler if no custom handlers were provided
        if self.event_handlers.is_empty() {
            upload_manager.add_event_handler(Arc::new(ConsoleEventHandler)).await;
        }

        info!("Application initialized successfully");

        Ok(VideoStreamingApp {
            config,
            upload_manager,
        })
    }

    async fn init_logging(&self) -> Result<(), UploadManagerError> {
        // For now, use a simple console logger
        // In a full implementation, this would be configurable based on the config
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();

        Ok(())
    }

    async fn load_configuration(&self) -> Result<AppConfig, UploadManagerError> {
        let mut config = if let Some(custom_config) = &self.custom_config {
            custom_config.clone()
        } else {
            // Load from file or create default
            let config_path = self.config_path.clone()
                .unwrap_or_else(|| PathBuf::from("video_uploader.toml"));

            let mut config_manager = ConfigManager::new(config_path);
            config_manager.load().await?;
            config_manager.get_config().clone()
        };

        // Apply environment overrides if enabled
        if self.enable_env_overrides {
            EnvConfigOverride::apply_overrides(&mut config);
        }

        Ok(config)
    }

    fn create_upload_config(&self, app_config: &AppConfig) -> Result<UploadConfig, UploadManagerError> {
        let config_manager = ConfigManager::new(PathBuf::new());
        let mut temp_manager = config_manager;
        temp_manager.update_config(app_config.clone())?;
        temp_manager.to_upload_config()
    }

    async fn create_upload_manager(&self, upload_config: UploadConfig) -> Result<UploadManager, UploadManagerError> {
        // Create components using dependency injection
        let file_reader = self.custom_file_reader.clone()
            .unwrap_or_else(|| Arc::new(DefaultFileStreamReader::new()));

        let segmenter = self.custom_segmenter.clone()
            .unwrap_or_else(|| Arc::new(DefaultVideoSegmenter::new()));

        let separator = self.custom_separator.clone()
            .unwrap_or_else(|| Arc::new(DefaultAudioVideoSeparator::new()));

        let transport = self.custom_transport.clone()
            .unwrap_or_else(|| Arc::new(DefaultQUICTransport::new()));

        let controller = self.custom_controller.clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(DefaultPlaybackController::new())));

        let monitor = self.custom_monitor.clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(DefaultPerformanceMonitor::new())));

        // Create upload manager with injected dependencies
        let upload_manager = UploadManager::with_components(
            upload_config,
            file_reader,
            segmenter,
            separator,
            transport,
            controller,
            monitor,
        );

        Ok(upload_manager)
    }
}

/// Main application structure
pub struct VideoStreamingApp {
    config: AppConfig,
    upload_manager: UploadManager,
}

impl VideoStreamingApp {
    /// Create a new application with default configuration
    pub async fn new() -> Result<Self, UploadManagerError> {
        AppBuilder::new().build().await
    }

    /// Create a new application with custom configuration file
    pub async fn with_config_file(config_path: PathBuf) -> Result<Self, UploadManagerError> {
        AppBuilder::new()
            .with_config_path(config_path)
            .build()
            .await
    }

    /// Create a new application with custom configuration
    pub async fn with_config(config: AppConfig) -> Result<Self, UploadManagerError> {
        AppBuilder::new()
            .with_config(config)
            .build()
            .await
    }

    /// Get reference to the upload manager
    pub fn upload_manager(&self) -> &UploadManager {
        &self.upload_manager
    }

    /// Get reference to the configuration
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Start the application and run until shutdown
    pub async fn run(&self) -> Result<(), UploadManagerError> {
        info!("Starting Video Streaming Uploader application");

        // Application main loop would go here
        // For now, we just keep the application alive
        tokio::signal::ctrl_c().await
            .map_err(|e| UploadManagerError::ConfigurationError { 
                message: format!("Failed to listen for shutdown signal: {}", e) 
            })?;

        info!("Shutdown signal received, stopping application");
        Ok(())
    }

    /// Graceful shutdown
    pub async fn shutdown(&self) -> Result<(), UploadManagerError> {
        info!("Shutting down application gracefully");

        // Stop any active uploads
        if self.upload_manager.is_running().await {
            self.upload_manager.stop_upload().await?;
        }

        info!("Application shutdown complete");
        Ok(())
    }
}

/// Application factory for creating pre-configured applications
pub struct AppFactory;

impl AppFactory {
    /// Create a development configuration application
    pub async fn create_development_app() -> Result<VideoStreamingApp, UploadManagerError> {
        let mut config = AppConfig::default();
        
        // Development-specific settings
        config.logging.level = "debug".to_string();
        config.monitoring.enabled = true;
        config.error_handling.enable_recovery = true;
        config.server.verify_certificates = false; // For local testing
        
        AppBuilder::new()
            .with_config(config)
            .build()
            .await
    }

    /// Create a production configuration application
    pub async fn create_production_app(config_path: PathBuf) -> Result<VideoStreamingApp, UploadManagerError> {
        AppBuilder::new()
            .with_config_path(config_path)
            .with_env_overrides(true)
            .build()
            .await
    }

    /// Create a testing configuration application
    pub async fn create_test_app() -> Result<VideoStreamingApp, UploadManagerError> {
        let mut config = AppConfig::default();
        
        // Test-specific settings
        config.logging.level = "trace".to_string();
        config.monitoring.enabled = false; // Disable for faster tests
        config.error_handling.max_retry_attempts = 1; // Fail fast in tests
        config.server.connection_timeout = std::time::Duration::from_secs(5);
        
        AppBuilder::new()
            .with_config(config)
            .with_env_overrides(false) // Don't use env vars in tests
            .build()
            .await
    }
}

/// Health check system for monitoring application status
pub struct HealthChecker {
    upload_manager: Arc<UploadManager>,
}

impl HealthChecker {
    pub fn new(upload_manager: Arc<UploadManager>) -> Self {
        Self { upload_manager }
    }

    /// Perform a comprehensive health check
    pub async fn check_health(&self) -> HealthStatus {
        let mut status = HealthStatus {
            overall: HealthState::Healthy,
            components: std::collections::HashMap::new(),
        };

        // Check upload manager status
        let upload_status = if self.upload_manager.is_running().await {
            HealthState::Healthy
        } else {
            HealthState::Idle
        };
        status.components.insert("upload_manager".to_string(), upload_status);

        // Check performance monitoring
        if let Ok(_stats) = self.upload_manager.get_performance_stats().await {
            status.components.insert("performance_monitor".to_string(), HealthState::Healthy);
        } else {
            status.components.insert("performance_monitor".to_string(), HealthState::Unhealthy);
            status.overall = HealthState::Degraded;
        }

        // Overall status is the worst component status
        for component_status in status.components.values() {
            match component_status {
                HealthState::Unhealthy => {
                    status.overall = HealthState::Unhealthy;
                    break;
                }
                HealthState::Degraded => {
                    if status.overall == HealthState::Healthy {
                        status.overall = HealthState::Degraded;
                    }
                }
                _ => {}
            }
        }

        status
    }
}

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub overall: HealthState,
    pub components: std::collections::HashMap<String, HealthState>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
    Idle,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthState::Healthy => write!(f, "healthy"),
            HealthState::Degraded => write!(f, "degraded"),
            HealthState::Unhealthy => write!(f, "unhealthy"),
            HealthState::Idle => write!(f, "idle"),
        }
    }
}