pub mod file_reader;
pub mod ffmpeg_cli_parser;
pub mod timeline_manager;
pub mod segmenter;
pub mod separator;
pub mod transport;
#[cfg(test)]
mod transport_test;
pub mod controller;
#[cfg(test)]
mod controller_test;
pub mod monitor;
#[cfg(test)]
mod monitor_test;
pub mod network_error_handler;
pub mod resource_monitor;
#[cfg(test)]
mod error_handling_integration_test;
#[cfg(test)]
mod seek_integration_test;
pub mod upload_manager;
pub mod config;
pub mod app;
pub mod types;
pub mod errors;
pub mod serde_helpers;
pub mod transmission_config;
pub mod mock_server;
pub mod on_demand_uploader;
pub mod live_encoder;
pub mod h264_encoder;

pub use types::*;
pub use errors::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_structure() {
        // Test that all core components can be instantiated
        let _file_reader = file_reader::DefaultFileStreamReader::new();
        let _segmenter = segmenter::DefaultVideoSegmenter::new();
        let _separator = separator::DefaultAudioVideoSeparator::new();
        let _transport = transport::DefaultQUICTransport::new();
        let _controller = controller::DefaultPlaybackController::new();
        let _monitor = monitor::DefaultPerformanceMonitor::new();
    }

    #[test]
    fn test_video_segment_creation() {
        use uuid::Uuid;
        use std::collections::HashMap;

        let segment = VideoSegment {
            id: Uuid::new_v4(),
            data: vec![1, 2, 3, 4],
            timestamp: 0.0,
            duration: 1.0,
            frame_count: 30,
            is_key_frame: true,
            metadata: SegmentMetadata {
                frame_indices: vec![0, 1, 2],
                key_frame_positions: vec![0],
                encoding_params: HashMap::new(),
            },
        };

        assert_eq!(segment.data.len(), 4);
        assert_eq!(segment.frame_count, 30);
        assert!(segment.is_key_frame);
    }

    #[test]
    fn test_audio_segment_creation() {
        use uuid::Uuid;

        let segment = AudioSegment {
            id: Uuid::new_v4(),
            data: vec![1, 2, 3, 4],
            timestamp: 0.0,
            duration: 1.0,
            sample_rate: 44100,
            channels: 2,
        };

        assert_eq!(segment.data.len(), 4);
        assert_eq!(segment.sample_rate, 44100);
        assert_eq!(segment.channels, 2);
    }

    #[test]
    fn test_error_types() {
        use crate::errors::*;

        let file_error = FileError::UnsupportedFormat {
            format: "avi".to_string(),
        };
        assert!(matches!(file_error, FileError::UnsupportedFormat { .. }));

        let segment_error = SegmentError::KeyFrameNotFound;
        assert!(matches!(segment_error, SegmentError::KeyFrameNotFound));

        let transport_error = TransportError::ConnectionFailed {
            reason: "timeout".to_string(),
        };
        assert!(matches!(transport_error, TransportError::ConnectionFailed { .. }));
    }
}