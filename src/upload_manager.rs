use std::path::Path;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, RwLock};
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::file_reader::{FileStreamReader, DefaultFileStreamReader};
use crate::segmenter::{VideoSegmenter, DefaultVideoSegmenter};
use crate::separator::{AudioVideoSeparator, DefaultAudioVideoSeparator};
use crate::transport::{QUICTransport, DefaultQUICTransport};
use crate::controller::{PlaybackController, DefaultPlaybackController};
use crate::monitor::{PerformanceMonitor, DefaultPerformanceMonitor};
use crate::network_error_handler::{NetworkErrorHandler, DefaultNetworkErrorHandler};
use crate::resource_monitor::DefaultSystemResourceMonitor;

use crate::types::{
    VideoFileInfo, VideoMetadata, TransmissionSession, TransmissionStatus, 
    QUICOptions, SegmentOptions, SegmentMode, StreamType, Segment,
    PerformanceStats, PerformanceReport, ExportFormat, NetworkConditions,
    ProtocolMessage, MessageType, ProtocolVersion, StatusCode, QUICConnection
};
use crate::errors::{
    FileError, SegmentError, TransportError, PlaybackError, 
    ExportError, UploadManagerError
};

/// Events that can be emitted by the UploadManager
#[derive(Debug, Clone)]
pub enum UploadEvent {
    SessionStarted { session_id: Uuid },
    FileOpened { file_info: VideoFileInfo },
    SegmentationStarted,
    TransmissionStarted,
    ProgressUpdate { progress: f64, current_position: f64 },
    PlaybackRateChanged { new_rate: f64 },
    SeekCompleted { position: f64 },
    NetworkConditionsChanged { conditions: NetworkConditions },
    ErrorOccurred { error: String },
    SessionCompleted { session_id: Uuid, report: PerformanceReport },
    SessionFailed { session_id: Uuid, error: String },
}

/// Configuration for the UploadManager
#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub server_address: SocketAddr,
    pub quic_options: QUICOptions,
    pub segment_options: SegmentOptions,
    pub max_concurrent_uploads: usize,
    pub enable_performance_monitoring: bool,
    pub enable_error_recovery: bool,
    pub protocol_version: ProtocolVersion,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            server_address: "127.0.0.1:4433".parse().unwrap(),
            quic_options: QUICOptions {
                max_concurrent_streams: 100,
                initial_max_data: 50 * 1024 * 1024, // 50MB - increased for large file transfers
                initial_max_stream_data: 5 * 1024 * 1024, // 5MB - increased to support large segments
                idle_timeout: Duration::from_secs(30),
            },
            segment_options: SegmentOptions {
                segment_mode: SegmentMode::Frame,
                target_size: Some(64 * 1024), // 64KB
                max_frames_per_segment: Some(30),
            },
            max_concurrent_uploads: 4,
            enable_performance_monitoring: true,
            enable_error_recovery: true,
            protocol_version: ProtocolVersion::CURRENT,
        }
    }
}

/// Event handler trait for receiving upload events
pub trait EventHandler: Send + Sync {
    fn handle_event(&self, event: UploadEvent);
}

/// Main upload manager that orchestrates all components
pub struct UploadManager {
    config: UploadConfig,
    file_reader: Arc<dyn FileStreamReader + Send + Sync>,
    segmenter: Arc<dyn VideoSegmenter + Send + Sync>,
    separator: Arc<DefaultAudioVideoSeparator>,
    transport: Arc<dyn QUICTransport + Send + Sync>,
    controller: Arc<Mutex<dyn PlaybackController + Send + Sync>>,
    monitor: Arc<Mutex<dyn PerformanceMonitor + Send + Sync>>,
    error_handler: Arc<DefaultNetworkErrorHandler>,
    resource_monitor: Arc<DefaultSystemResourceMonitor>,
    
    // Session management
    current_session: Arc<RwLock<Option<TransmissionSession>>>,
    event_handlers: Arc<RwLock<Vec<Arc<dyn EventHandler>>>>,
    
    // State tracking
    is_running: Arc<RwLock<bool>>,
    connection: Arc<Mutex<Option<crate::types::QUICConnection>>>,
}

impl UploadManager {
    /// Create a new UploadManager with default components
    pub fn new(config: UploadConfig) -> Self {
        Self {
            config,
            file_reader: Arc::new(DefaultFileStreamReader::new()),
            segmenter: Arc::new(DefaultVideoSegmenter::new()),
            separator: Arc::new(DefaultAudioVideoSeparator::new()),
            transport: Arc::new(DefaultQUICTransport::new()),
            controller: Arc::new(Mutex::new(DefaultPlaybackController::new())),
            monitor: Arc::new(Mutex::new(DefaultPerformanceMonitor::new())),
            error_handler: Arc::new(DefaultNetworkErrorHandler::new()),
            resource_monitor: Arc::new(DefaultSystemResourceMonitor::new()),
            current_session: Arc::new(RwLock::new(None)),
            event_handlers: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// Create a new UploadManager with custom components
    pub fn with_components(
        config: UploadConfig,
        file_reader: Arc<dyn FileStreamReader + Send + Sync>,
        segmenter: Arc<dyn VideoSegmenter + Send + Sync>,
        separator: Arc<DefaultAudioVideoSeparator>,
        transport: Arc<dyn QUICTransport + Send + Sync>,
        controller: Arc<Mutex<dyn PlaybackController + Send + Sync>>,
        monitor: Arc<Mutex<dyn PerformanceMonitor + Send + Sync>>,
    ) -> Self {
        Self {
            config,
            file_reader,
            segmenter,
            separator,
            transport,
            controller,
            monitor,
            error_handler: Arc::new(DefaultNetworkErrorHandler::new()),
            resource_monitor: Arc::new(DefaultSystemResourceMonitor::new()),
            current_session: Arc::new(RwLock::new(None)),
            event_handlers: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// Add an event handler to receive upload events
    pub async fn add_event_handler(&self, handler: Arc<dyn EventHandler>) {
        let mut handlers = self.event_handlers.write().await;
        handlers.push(handler);
    }

    /// Start a new upload session
    pub async fn start_upload(&self, file_path: &Path) -> Result<Uuid, UploadManagerError> {
        // Check if already running
        if *self.is_running.read().await {
            return Err(UploadManagerError::SessionAlreadyActive);
        }

        // Validate file and get metadata
        let file_info = self.validate_and_get_file_info(file_path).await?;
        
        // Create new session
        let session_id = Uuid::new_v4();
        let session = TransmissionSession {
            id: session_id,
            video_metadata: self.create_video_metadata(file_path, &file_info).await?,
            start_time: SystemTime::now(),
            end_time: None,
            status: TransmissionStatus::Preparing,
            progress: 0.0,
            current_position: 0.0,
            playback_rate: 1.0,
            total_segments: 0,
            transmitted_segments: 0,
            errors: Vec::new(),
        };

        // Store session
        {
            let mut current_session = self.current_session.write().await;
            *current_session = Some(session);
        }

        // Set running state
        {
            let mut is_running = self.is_running.write().await;
            *is_running = true;
        }

        // Emit event
        self.emit_event(UploadEvent::SessionStarted { session_id }).await;
        self.emit_event(UploadEvent::FileOpened { file_info }).await;

        // Start the upload process in a separate task
        // Note: In a real implementation, we'd need to handle this differently
        // For now, we'll run it synchronously to avoid lifetime issues
        if let Err(e) = self.run_upload_process(file_path.to_path_buf()).await {
            self.emit_event(UploadEvent::SessionFailed { 
                session_id, 
                error: e.to_string() 
            }).await;
        }

        Ok(session_id)
    }

    /// Stop the current upload session
    pub async fn stop_upload(&self) -> Result<(), UploadManagerError> {
        let mut is_running = self.is_running.write().await;
        if !*is_running {
            return Err(UploadManagerError::NoActiveSession);
        }

        *is_running = false;

        // Update session status
        if let Some(session) = self.current_session.write().await.as_mut() {
            session.status = TransmissionStatus::Paused;
        }

        // Close connection if exists
        if let Some(connection) = self.connection.lock().await.take() {
            // Connection will be dropped and closed automatically
            drop(connection);
        }

        Ok(())
    }

    /// Seek to a specific position in the video
    pub async fn seek(&self, position: f64) -> Result<(), UploadManagerError> {
        if !*self.is_running.read().await {
            return Err(UploadManagerError::NoActiveSession);
        }

        // Update controller
        {
            let mut controller = self.controller.lock().await;
            controller.seek(position).await
                .map_err(|e| UploadManagerError::PlaybackError(e))?;
        }

        // Update session position
        if let Some(session) = self.current_session.write().await.as_mut() {
            session.current_position = position;
        }

        self.emit_event(UploadEvent::SeekCompleted { position }).await;

        Ok(())
    }

    /// Set playback rate
    pub async fn set_playback_rate(&self, rate: f64) -> Result<(), UploadManagerError> {
        if !*self.is_running.read().await {
            return Err(UploadManagerError::NoActiveSession);
        }

        // Update controller
        {
            let mut controller = self.controller.lock().await;
            controller.set_playback_rate(rate).await
                .map_err(|e| UploadManagerError::PlaybackError(e))?;
        }

        // Update session rate
        if let Some(session) = self.current_session.write().await.as_mut() {
            session.playback_rate = rate;
        }

        self.emit_event(UploadEvent::PlaybackRateChanged { new_rate: rate }).await;

        Ok(())
    }

    /// Get current performance statistics
    pub async fn get_performance_stats(&self) -> Result<PerformanceStats, UploadManagerError> {
        if !self.config.enable_performance_monitoring {
            return Err(UploadManagerError::MonitoringDisabled);
        }

        let monitor = self.monitor.lock().await;
        Ok(monitor.get_current_stats())
    }

    /// Generate performance report
    pub async fn generate_performance_report(&self) -> Result<PerformanceReport, UploadManagerError> {
        if !self.config.enable_performance_monitoring {
            return Err(UploadManagerError::MonitoringDisabled);
        }

        let monitor = self.monitor.lock().await;
        Ok(monitor.generate_report())
    }

    /// Export performance data
    pub async fn export_performance_data(&self, format: ExportFormat) -> Result<String, UploadManagerError> {
        if !self.config.enable_performance_monitoring {
            return Err(UploadManagerError::MonitoringDisabled);
        }

        let monitor = self.monitor.lock().await;
        monitor.export_data(format)
            .map_err(|e| UploadManagerError::ExportError(e))
    }

    /// Get current session information
    pub async fn get_current_session(&self) -> Option<TransmissionSession> {
        self.current_session.read().await.clone()
    }

    /// Check if upload is currently running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    // Private helper methods

    async fn validate_and_get_file_info(&self, file_path: &Path) -> Result<VideoFileInfo, UploadManagerError> {
        // Validate format first
        self.file_reader.validate_format(file_path).await
            .map_err(|e| UploadManagerError::FileError(e))?;

        // Open file and get info
        let mut file = self.file_reader.open_file(file_path).await
            .map_err(|e| UploadManagerError::FileError(e))?;
        
        let file_info = self.file_reader.get_file_info(&mut file).await
            .map_err(|e| UploadManagerError::FileError(e))?;

        // Close file
        self.file_reader.close_file(file).await
            .map_err(|e| UploadManagerError::FileError(e))?;

        Ok(file_info)
    }

    async fn create_video_metadata(&self, file_path: &Path, file_info: &VideoFileInfo) -> Result<VideoMetadata, UploadManagerError> {
        use std::fs;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let metadata = fs::metadata(file_path)
            .map_err(|e| UploadManagerError::FileError(FileError::Io(e)))?;

        // Create a simple checksum (in production, use a proper hash)
        let mut hasher = DefaultHasher::new();
        file_path.hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        let checksum = format!("{:x}", hasher.finish());

        Ok(VideoMetadata {
            file_path: file_path.to_path_buf(),
            file_size: metadata.len(),
            duration: file_info.duration,
            resolution: file_info.resolution.clone(),
            codec: crate::types::VideoCodec {
                name: file_info.codec.clone(),
                profile: "main".to_string(), // Default profile
                level: "4.0".to_string(),    // Default level
                color_space: "yuv420p".to_string(),
                bit_depth: 8,
            },
            frame_rate: file_info.frame_rate,
            bit_rate: file_info.bit_rate,
            audio_tracks: if file_info.has_audio {
                vec![crate::types::AudioTrackInfo {
                    index: 0,
                    codec: "aac".to_string(),
                    sample_rate: 44100,
                    channels: 2,
                    bit_rate: 128000,
                    language: None,
                }]
            } else {
                Vec::new()
            },
            created_at: SystemTime::now(),
            checksum,
        })
    }

    async fn emit_event(&self, event: UploadEvent) {
        let handlers = self.event_handlers.read().await;
        for handler in handlers.iter() {
            handler.handle_event(event.clone());
        }
    }
}

impl UploadManager {
    /// Main upload process that orchestrates all components
    async fn run_upload_process(&self, file_path: std::path::PathBuf) -> Result<(), UploadManagerError> {
        // Start performance monitoring if enabled
        if self.config.enable_performance_monitoring {
            let mut monitor = self.monitor.lock().await;
            monitor.start_monitoring();
        }

        // Update session status
        {
            let mut session = self.current_session.write().await;
            if let Some(ref mut session) = session.as_mut() {
                session.status = TransmissionStatus::Active;
            }
        }

        // Establish QUIC connection
        let connection = self.establish_connection().await?;
        
        // Store connection
        {
            let mut conn_guard = self.connection.lock().await;
            *conn_guard = Some(connection);
        }
        
        // Get connection for use
        let mut connection = {
            let conn_guard = self.connection.lock().await;
            // Create a new reference to the connection
            QUICConnection {
                id: conn_guard.as_ref().unwrap().id,
                remote_address: conn_guard.as_ref().unwrap().remote_address,
                established_at: conn_guard.as_ref().unwrap().established_at,
                stats: conn_guard.as_ref().unwrap().stats.clone(),
                inner: conn_guard.as_ref().unwrap().inner.clone(),
                config: conn_guard.as_ref().unwrap().config.clone(),
            }
        };

        // Open and process file
        let mut file = self.file_reader.open_file(&file_path).await?;
        
        // Create input stream from file
        let file_stream = self.create_file_stream(&mut file).await?;
        
        self.emit_event(UploadEvent::SegmentationStarted).await;
        
        // Segment the video
        let segment_stream = self.segmenter.segment_stream(
            Box::new(file_stream),
            self.config.segment_options.clone(),
        ).await;

        self.emit_event(UploadEvent::TransmissionStarted).await;

        // Process segments and transmit
        let mut segment_count = 0u64;
        let mut transmitted_count = 0u64;
        
        tokio::pin!(segment_stream);
        
        while let Some(segment_result) = segment_stream.next().await {
            // Check if we should continue running
            if !*self.is_running.read().await {
                break;
            }

            match segment_result {
                Ok(video_segment) => {
                    segment_count += 1;
                    
                    // Update total segments in session
                    {
                        let mut session = self.current_session.write().await;
                        if let Some(ref mut session) = session.as_mut() {
                            session.total_segments = segment_count;
                        }
                    }

                    // Check if we need to separate audio/video
                    if self.should_separate_streams(&video_segment).await {
                        // Process with audio/video separation
                        if let Err(e) = self.process_with_separation(video_segment, &mut connection).await {
                            self.handle_transmission_error(e).await;
                            continue;
                        }
                    } else {
                        // Process video-only
                        if let Err(e) = self.transmit_video_segment(video_segment, &mut connection).await {
                            self.handle_transmission_error(e).await;
                            continue;
                        }
                    }
                    
                    transmitted_count += 1;
                    
                    // Update progress
                    let progress = if segment_count > 0 {
                        (transmitted_count as f64 / segment_count as f64) * 100.0
                    } else {
                        0.0
                    };
                    
                    // Update session progress
                    {
                        let mut session = self.current_session.write().await;
                        if let Some(ref mut session) = session.as_mut() {
                            session.transmitted_segments = transmitted_count;
                            session.progress = progress;
                        }
                    }
                    
                    self.emit_event(UploadEvent::ProgressUpdate { 
                        progress, 
                        current_position: 0.0 // TODO: Calculate actual position
                    }).await;
                    
                    // Monitor network conditions
                    if let Ok(conditions) = self.transport.monitor_and_adjust(&mut connection).await {
                        self.emit_event(UploadEvent::NetworkConditionsChanged { conditions }).await;
                    }
                }
                Err(e) => {
                    self.handle_segmentation_error(e).await;
                }
            }
        }

        // Close file
        self.file_reader.close_file(file).await?;

        // Complete session
        self.complete_session().await?;

        Ok(())
    }

    async fn establish_connection(&self) -> Result<crate::types::QUICConnection, UploadManagerError> {
        self.transport.connect(
            self.config.server_address,
            self.config.quic_options.clone(),
        ).await.map_err(|e| UploadManagerError::TransportError(e))
    }

    async fn create_file_stream(&self, file: &mut tokio::fs::File) -> Result<impl tokio_stream::Stream<Item = Vec<u8>>, UploadManagerError> {
        use tokio_stream::wrappers::UnboundedReceiverStream;
        use tokio::sync::mpsc;
        
        let (tx, rx) = mpsc::unbounded_channel();
        
        // Read file in chunks
        let chunk_size = 64 * 1024; // 64KB chunks
        let mut buffer = vec![0u8; chunk_size];
        
        loop {
            match self.file_reader.read_chunk(file, chunk_size).await {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        break; // End of file
                    }
                    if tx.send(chunk).is_err() {
                        break; // Receiver dropped
                    }
                }
                Err(e) => {
                    return Err(UploadManagerError::FileError(e));
                }
            }
        }
        
        Ok(UnboundedReceiverStream::new(rx))
    }

    async fn should_separate_streams(&self, _video_segment: &crate::types::VideoSegment) -> bool {
        // Check if the current session has audio tracks
        if let Some(session) = self.current_session.read().await.as_ref() {
            !session.video_metadata.audio_tracks.is_empty()
        } else {
            false
        }
    }

    async fn process_with_separation(
        &self,
        video_segment: crate::types::VideoSegment,
        connection: &mut crate::types::QUICConnection,
    ) -> Result<(), UploadManagerError> {
        // Create a stream from the single segment for separation
        let segment_stream = tokio_stream::once(video_segment.data.clone());
        
        // Separate audio and video
        let separated = self.separator.separate_streams(Box::new(segment_stream)).await?;
        
        // Create video and audio streams
        let mut video_stream = self.transport.create_stream(connection, StreamType::Video).await?;
        let mut audio_stream = self.transport.create_stream(connection, StreamType::Audio).await?;
        
        // Configure streams for optimal transmission
        self.transport.configure_video_stream(connection, &mut video_stream).await?;
        self.transport.configure_audio_stream(connection, &mut audio_stream).await?;
        
        // Transmit video segment
        self.transmit_video_segment(video_segment, connection).await?;
        
        // Process audio segments from separated stream
        let mut audio_stream = separated.audio_stream;
        tokio::pin!(audio_stream);
        while let Some(audio_segment) = audio_stream.next().await {
            self.transmit_audio_segment(audio_segment, connection).await?;
        }
        
        Ok(())
    }

    async fn transmit_video_segment(
        &self,
        segment: crate::types::VideoSegment,
        connection: &mut crate::types::QUICConnection,
    ) -> Result<(), UploadManagerError> {
        let start_time = std::time::Instant::now();
        
        // Send segment
        self.transport.send_segment(connection, Segment::Video(segment.clone())).await?;
        
        // Record performance if monitoring is enabled
        if self.config.enable_performance_monitoring {
            let latency = start_time.elapsed();
            let mut monitor = self.monitor.lock().await;
            monitor.record_transmission(&Segment::Video(segment), latency);
        }
        
        Ok(())
    }

    async fn transmit_audio_segment(
        &self,
        segment: crate::types::AudioSegment,
        connection: &mut crate::types::QUICConnection,
    ) -> Result<(), UploadManagerError> {
        let start_time = std::time::Instant::now();
        
        // Send segment
        self.transport.send_segment(connection, Segment::Audio(segment.clone())).await?;
        
        // Record performance if monitoring is enabled
        if self.config.enable_performance_monitoring {
            let latency = start_time.elapsed();
            let mut monitor = self.monitor.lock().await;
            monitor.record_transmission(&Segment::Audio(segment), latency);
        }
        
        Ok(())
    }

    async fn handle_transmission_error(&self, error: UploadManagerError) {
        // Log error to session
        if let Some(session) = self.current_session.write().await.as_mut() {
            session.errors.push(crate::types::TransmissionError {
                timestamp: SystemTime::now(),
                error_type: match &error {
                    UploadManagerError::NetworkError(_) => crate::types::ErrorType::Network,
                    UploadManagerError::FileError(_) => crate::types::ErrorType::File,
                    UploadManagerError::TransportError(_) => crate::types::ErrorType::Network,
                    _ => crate::types::ErrorType::Server,
                },
                message: error.to_string(),
                segment_id: None,
                retry_count: 0,
                resolved: false,
            });
        }

        // Emit error event
        self.emit_event(UploadEvent::ErrorOccurred { 
            error: error.to_string() 
        }).await;

        // Attempt recovery if enabled
        if self.config.enable_error_recovery {
            // TODO: Implement error recovery logic
        }
    }

    async fn handle_segmentation_error(&self, error: SegmentError) {
        self.handle_transmission_error(UploadManagerError::SegmentError(error)).await;
    }

    async fn complete_session(&self) -> Result<(), UploadManagerError> {
        let session_id = {
            let mut session = self.current_session.write().await;
            if let Some(ref mut session) = session.as_mut() {
                session.status = TransmissionStatus::Completed;
                session.end_time = Some(SystemTime::now());
                session.progress = 100.0;
                session.id
            } else {
                return Err(UploadManagerError::NoActiveSession);
            }
        };

        // Generate performance report if monitoring is enabled
        let report = if self.config.enable_performance_monitoring {
            let monitor = self.monitor.lock().await;
            monitor.generate_report()
        } else {
            // Create a minimal report
            crate::types::PerformanceReport {
                session_id,
                start_time: SystemTime::now(),
                end_time: SystemTime::now(),
                total_duration: Duration::from_secs(0),
                stats: crate::types::PerformanceStats {
                    current_upload_speed: 0.0,
                    average_upload_speed: 0.0,
                    current_latency: Duration::from_millis(0),
                    average_latency: Duration::from_millis(0),
                    total_bytes_transmitted: 0,
                    segments_transmitted: 0,
                },
                peak_upload_speed: 0.0,
                latency_distribution: Vec::new(),
                anomalies: Vec::new(),
            }
        };

        // Set running state to false
        {
            let mut is_running = self.is_running.write().await;
            *is_running = false;
        }

        // Emit completion event
        self.emit_event(UploadEvent::SessionCompleted { session_id, report }).await;

        Ok(())
    }
}

/// Simple console event handler for demonstration
pub struct ConsoleEventHandler;

impl EventHandler for ConsoleEventHandler {
    fn handle_event(&self, event: UploadEvent) {
        match event {
            UploadEvent::SessionStarted { session_id } => {
                println!("Upload session started: {}", session_id);
            }
            UploadEvent::FileOpened { file_info } => {
                println!("File opened: {}x{}, {:.2}s duration", 
                    file_info.resolution.width, 
                    file_info.resolution.height, 
                    file_info.duration);
            }
            UploadEvent::ProgressUpdate { progress, current_position } => {
                println!("Progress: {:.1}% at position {:.2}s", progress, current_position);
            }
            UploadEvent::SessionCompleted { session_id, .. } => {
                println!("Upload session completed: {}", session_id);
            }
            UploadEvent::ErrorOccurred { error } => {
                eprintln!("Upload error: {}", error);
            }
            _ => {
                println!("Upload event: {:?}", event);
            }
        }
    }
}