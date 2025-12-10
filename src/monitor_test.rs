use crate::monitor::{PerformanceMonitor, DefaultPerformanceMonitor};
use crate::types::{Segment, VideoSegment, AudioSegment, SegmentMetadata, ExportFormat};
use std::time::Duration;
use std::collections::HashMap;
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_video_segment() -> VideoSegment {
        VideoSegment {
            id: Uuid::new_v4(),
            data: vec![0u8; 1024], // 1KB of test data
            timestamp: 0.0,
            duration: 1.0,
            frame_count: 30,
            is_key_frame: true,
            metadata: SegmentMetadata {
                frame_indices: vec![0, 1, 2],
                key_frame_positions: vec![0],
                encoding_params: HashMap::new(),
            },
        }
    }

    fn create_test_audio_segment() -> AudioSegment {
        AudioSegment {
            id: Uuid::new_v4(),
            data: vec![0u8; 512], // 512B of test data
            timestamp: 0.0,
            duration: 1.0,
            sample_rate: 44100,
            channels: 2,
        }
    }

    #[test]
    fn test_performance_monitor_creation() {
        let monitor = DefaultPerformanceMonitor::new();
        let stats = monitor.get_current_stats();
        
        assert_eq!(stats.current_upload_speed, 0.0);
        assert_eq!(stats.average_upload_speed, 0.0);
        assert_eq!(stats.total_bytes_transmitted, 0);
        assert_eq!(stats.segments_transmitted, 0);
    }

    #[test]
    fn test_start_monitoring() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let stats = monitor.get_current_stats();
        assert_eq!(stats.total_bytes_transmitted, 0);
        assert_eq!(stats.segments_transmitted, 0);
    }

    #[test]
    fn test_record_transmission_video() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let latency = Duration::from_millis(50);
        
        monitor.record_transmission(&segment, latency);
        
        let stats = monitor.get_current_stats();
        assert_eq!(stats.total_bytes_transmitted, 1024);
        assert_eq!(stats.segments_transmitted, 1);
        assert_eq!(stats.current_latency, latency);
    }

    #[test]
    fn test_record_transmission_audio() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let audio_segment = create_test_audio_segment();
        let segment = Segment::Audio(audio_segment);
        let latency = Duration::from_millis(25);
        
        monitor.record_transmission(&segment, latency);
        
        let stats = monitor.get_current_stats();
        assert_eq!(stats.total_bytes_transmitted, 512);
        assert_eq!(stats.segments_transmitted, 1);
        assert_eq!(stats.current_latency, latency);
    }

    #[test]
    fn test_multiple_transmissions() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        // Record multiple transmissions
        for i in 0..5 {
            let video_segment = create_test_video_segment();
            let segment = Segment::Video(video_segment);
            let latency = Duration::from_millis(50 + i * 10);
            
            monitor.record_transmission(&segment, latency);
        }
        
        let stats = monitor.get_current_stats();
        assert_eq!(stats.total_bytes_transmitted, 5 * 1024);
        assert_eq!(stats.segments_transmitted, 5);
        
        // Average latency should be around 70ms (50, 60, 70, 80, 90)
        let expected_avg = Duration::from_millis(70);
        let actual_avg = stats.average_latency;
        assert!((actual_avg.as_millis() as i64 - expected_avg.as_millis() as i64).abs() <= 5);
    }

    #[test]
    fn test_generate_report() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let latency = Duration::from_millis(100);
        
        monitor.record_transmission(&segment, latency);
        
        let report = monitor.generate_report();
        
        assert_eq!(report.stats.total_bytes_transmitted, 1024);
        assert_eq!(report.stats.segments_transmitted, 1);
        assert!(!report.latency_distribution.is_empty());
        assert_eq!(report.latency_distribution[0], latency);
    }

    #[test]
    fn test_export_json() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let latency = Duration::from_millis(75);
        
        monitor.record_transmission(&segment, latency);
        
        let json_result = monitor.export_data(ExportFormat::Json);
        assert!(json_result.is_ok());
        
        let json_data = json_result.unwrap();
        assert!(json_data.contains("session_id"));
        assert!(json_data.contains("total_bytes_transmitted"));
        assert!(json_data.contains("1024")); // Our test data size
    }

    #[test]
    fn test_export_csv() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let latency = Duration::from_millis(60);
        
        monitor.record_transmission(&segment, latency);
        
        let csv_result = monitor.export_data(ExportFormat::Csv);
        assert!(csv_result.is_ok());
        
        let csv_data = csv_result.unwrap();
        assert!(csv_data.contains("timestamp,segment_size,latency_ms"));
        assert!(csv_data.contains("1024")); // Our test data size
        assert!(csv_data.contains("60")); // Our test latency
    }

    #[test]
    fn test_anomaly_detection() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        // Record normal transmissions
        for _ in 0..5 {
            let video_segment = create_test_video_segment();
            let segment = Segment::Video(video_segment);
            let latency = Duration::from_millis(50);
            monitor.record_transmission(&segment, latency);
        }
        
        // Record an anomalous transmission with high latency
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let high_latency = Duration::from_millis(500); // 10x normal latency
        monitor.record_transmission(&segment, high_latency);
        
        let report = monitor.generate_report();
        
        // Should detect the high latency anomaly
        assert!(!report.anomalies.is_empty());
        assert!(report.anomalies.iter().any(|a| matches!(a.anomaly_type, crate::types::AnomalyType::HighLatency)));
    }

    #[test]
    fn test_upload_speed_calculation() {
        let mut monitor = DefaultPerformanceMonitor::new();
        monitor.start_monitoring();
        
        // Simulate some time passing and data transmission
        std::thread::sleep(Duration::from_millis(100));
        
        let video_segment = create_test_video_segment();
        let segment = Segment::Video(video_segment);
        let latency = Duration::from_millis(50);
        
        monitor.record_transmission(&segment, latency);
        
        let stats = monitor.get_current_stats();
        
        // Upload speed should be calculated (bytes per second)
        assert!(stats.average_upload_speed > 0.0);
        assert!(stats.current_upload_speed >= 0.0);
    }
}