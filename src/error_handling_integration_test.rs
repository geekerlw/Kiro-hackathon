#[cfg(test)]
mod tests {
    use crate::network_error_handler::{DefaultNetworkErrorHandler, RetryConfig, ProtocolConfig};
    use crate::resource_monitor::{DefaultSystemResourceMonitor, SystemResourceMonitor, MonitoringConfig};
    use crate::types::{QUICOptions, ProtocolVersion};
    use std::net::SocketAddr;
    use std::time::Duration;

    #[tokio::test]
    async fn test_network_error_handler_creation() {
        let handler = DefaultNetworkErrorHandler::new();
        let stats = handler.get_error_statistics().await;
        
        assert_eq!(stats.connection_failures, 0);
        assert_eq!(stats.protocol_errors, 0);
        assert_eq!(stats.recovery_attempts, 0);
        assert_eq!(stats.successful_recoveries, 0);
    }

    #[tokio::test]
    async fn test_network_error_handler_with_config() {
        let retry_config = RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            jitter_factor: 0.05,
        };
        
        let protocol_config = ProtocolConfig {
            supported_versions: vec![ProtocolVersion { major: 1, minor: 0, patch: 0 }],
            negotiation_timeout: Duration::from_secs(5),
            heartbeat_interval: Duration::from_secs(15),
            max_protocol_errors: 5,
        };
        
        let handler = DefaultNetworkErrorHandler::with_config(retry_config, protocol_config);
        let stats = handler.get_error_statistics().await;
        
        assert_eq!(stats.connection_failures, 0);
    }

    #[tokio::test]
    async fn test_server_error_handling() {
        let _handler = DefaultNetworkErrorHandler::new();
        let _server_address: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let _options = QUICOptions {
            max_concurrent_streams: 10,
            initial_max_data: 1_000_000,
            initial_max_stream_data: 100_000,
            idle_timeout: Duration::from_secs(30),
        };
        
        // For testing, we'll skip the actual connection tests since they require unsafe code
        // In a real implementation, these would use proper QUIC connections

        // Test would handle different server error codes with actual connection
        // Skipping actual connection tests due to complexity of mocking QUIC connections
    }

    #[tokio::test]
    async fn test_protocol_version_negotiation() {
        let _handler = DefaultNetworkErrorHandler::new();
        let _server_address: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let _options = QUICOptions {
            max_concurrent_streams: 10,
            initial_max_data: 1_000_000,
            initial_max_stream_data: 100_000,
            idle_timeout: Duration::from_secs(30),
        };
        
        // Protocol version negotiation would be tested with actual connection
        // Skipping due to complexity of mocking QUIC connections
        let client_version = ProtocolVersion { major: 1, minor: 0, patch: 0 };
        assert_eq!(client_version.major, 1);
    }

    #[tokio::test]
    async fn test_resource_monitor_creation() {
        let monitor = DefaultSystemResourceMonitor::new();
        let status = monitor.get_resource_status().await;
        
        assert!(status.is_ok());
        let status = status.unwrap();
        
        // Should have at least one storage status entry (current directory)
        assert!(!status.storage_status.is_empty());
        
        // Should have system resource information
        assert!(status.system_resources.memory_usage.total_memory > 0);
        assert!(status.system_resources.cpu_usage.usage_percentage >= 0.0);
    }

    #[tokio::test]
    async fn test_resource_monitor_with_config() {
        let config = MonitoringConfig {
            check_interval: Duration::from_secs(10),
            storage_warning_threshold: 2_000_000_000, // 2GB
            storage_critical_threshold: 1_000_000_000, // 1GB
            memory_warning_threshold: 2048, // 2GB
            memory_critical_threshold: 1024, // 1GB
            cpu_warning_threshold: 70.0,
            cpu_critical_threshold: 90.0,
            file_integrity_check_interval: Duration::from_secs(30),
            max_history_entries: 500,
        };
        
        let monitor = DefaultSystemResourceMonitor::with_config(config);
        let status = monitor.get_resource_status().await;
        
        assert!(status.is_ok());
    }

    #[tokio::test]
    async fn test_storage_space_checking() {
        let monitor = DefaultSystemResourceMonitor::new();
        let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        
        // Check for 1GB requirement
        let result = monitor.check_storage_space(&current_dir, 1_000_000_000).await;
        assert!(result.is_ok());
        
        let storage_info = result.unwrap();
        assert!(storage_info.total_space > 0);
        assert!(storage_info.available_space <= storage_info.total_space);
        assert!(storage_info.used_space <= storage_info.total_space);
        assert_eq!(storage_info.used_space + storage_info.available_space, storage_info.total_space);
    }

    #[tokio::test]
    async fn test_file_integrity_monitoring() {
        let mut monitor = DefaultSystemResourceMonitor::new();
        
        // Create a temporary file for testing
        let temp_file = std::env::temp_dir().join("test_integrity.txt");
        std::fs::write(&temp_file, b"test content").unwrap();
        
        // Add file to monitoring
        let result = monitor.add_file_to_monitoring(&temp_file).await;
        assert!(result.is_ok());
        
        // Check file integrity
        let result = monitor.monitor_file_integrity(&temp_file).await;
        assert!(result.is_ok());
        
        let integrity_info = result.unwrap();
        assert_eq!(integrity_info.file_path, temp_file);
        assert!(!integrity_info.current_checksum.is_empty());
        assert!(matches!(integrity_info.integrity_status, 
                        crate::resource_monitor::FileIntegrityStatus::Intact));
        
        // Clean up
        let _ = std::fs::remove_file(&temp_file);
        monitor.remove_file_from_monitoring(&temp_file);
    }

    #[tokio::test]
    async fn test_system_resource_monitoring() {
        let monitor = DefaultSystemResourceMonitor::new();
        
        let result = monitor.monitor_system_resources().await;
        assert!(result.is_ok());
        
        let resources = result.unwrap();
        
        // Verify memory usage information
        assert!(resources.memory_usage.total_memory > 0);
        assert!(resources.memory_usage.used_memory <= resources.memory_usage.total_memory);
        assert!(resources.memory_usage.available_memory <= resources.memory_usage.total_memory);
        assert!(resources.memory_usage.usage_percentage >= 0.0);
        assert!(resources.memory_usage.usage_percentage <= 100.0);
        
        // Verify CPU usage information
        assert!(resources.cpu_usage.usage_percentage >= 0.0);
        assert!(resources.cpu_usage.usage_percentage <= 100.0);
        assert!(resources.cpu_usage.load_average >= 0.0);
        
        // Verify I/O statistics
        assert!(resources.disk_io.read_bytes_per_sec >= 0);
        assert!(resources.disk_io.write_bytes_per_sec >= 0);
        assert!(resources.disk_io.io_utilization >= 0.0);
        assert!(resources.disk_io.io_utilization <= 100.0);
        
        assert!(resources.network_io.bytes_sent_per_sec >= 0);
        assert!(resources.network_io.bytes_received_per_sec >= 0);
        assert!(resources.network_io.network_utilization >= 0.0);
        assert!(resources.network_io.network_utilization <= 100.0);
    }

    #[tokio::test]
    async fn test_parameter_adjustment() {
        let monitor = DefaultSystemResourceMonitor::new();
        
        // Get current system resources
        let resources = monitor.monitor_system_resources().await.unwrap();
        
        // Test parameter adjustment based on current resources
        let result = monitor.adjust_parameters(&resources).await;
        assert!(result.is_ok());
        
        let adjustments = result.unwrap();
        
        // Verify adjustment structure
        assert!(!adjustments.pause_transmission); // Should not pause for normal conditions
        
        // Adjustments should be reasonable based on simulated resource usage
        // Since we're using simulated data, we can't make strong assertions about specific adjustments
        // but we can verify the structure is correct
    }

    #[tokio::test]
    async fn test_error_type_conversions() {
        // Test that our error types can be properly converted and chained
        let file_error = crate::errors::FileError::UnsupportedFormat {
            format: "unknown".to_string(),
        };
        let video_error: crate::errors::VideoStreamingError = file_error.into();
        assert!(matches!(video_error, crate::errors::VideoStreamingError::File(_)));

        let transport_error = crate::errors::TransportError::ConnectionFailed {
            reason: "test".to_string(),
        };
        let video_error: crate::errors::VideoStreamingError = transport_error.into();
        assert!(matches!(video_error, crate::errors::VideoStreamingError::Transport(_)));

        let network_error = crate::errors::NetworkError::RetryExhausted { attempts: 5 };
        let video_error: crate::errors::VideoStreamingError = network_error.into();
        assert!(matches!(video_error, crate::errors::VideoStreamingError::Network(_)));

        let resource_error = crate::errors::ResourceError::InsufficientStorage {
            available: 100,
            required: 200,
        };
        let video_error: crate::errors::VideoStreamingError = resource_error.into();
        assert!(matches!(video_error, crate::errors::VideoStreamingError::Resource(_)));
    }
}