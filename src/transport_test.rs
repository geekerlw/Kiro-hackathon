#[cfg(test)]
mod tests {
    use crate::transport::DefaultQUICTransport;
    use crate::types::*;

    #[tokio::test]
    async fn test_quic_transport_creation() {
        let _transport = DefaultQUICTransport::new();
        
        // Test that we can create the transport
        assert!(true); // Basic creation test
    }

    #[test]
    fn test_protocol_version_compatibility() {
        let v1_0_0 = ProtocolVersion { major: 1, minor: 0, patch: 0 };
        let v1_1_0 = ProtocolVersion { major: 1, minor: 1, patch: 0 };
        let v2_0_0 = ProtocolVersion { major: 2, minor: 0, patch: 0 };

        assert!(v1_0_0.is_compatible(&v1_1_0));
        assert!(!v1_0_0.is_compatible(&v2_0_0));
        assert!(!v2_0_0.is_compatible(&v1_0_0));
    }

    #[test]
    fn test_recovery_state_creation() {
        let recovery_state = RecoveryState::new();
        assert_eq!(recovery_state.retry_count, 0);
        assert_eq!(recovery_state.max_retries, 5);
        
        let adaptive_recovery = RecoveryState::with_strategy(RecoveryStrategy::AdaptiveBackoff);
        assert!(matches!(adaptive_recovery.recovery_strategy, RecoveryStrategy::AdaptiveBackoff));
    }

    #[test]
    fn test_quic_options_creation() {
        let options = QUICOptions {
            max_concurrent_streams: 100,
            initial_max_data: 10_000_000,
            initial_max_stream_data: 1_000_000,
            idle_timeout: std::time::Duration::from_secs(30),
        };

        assert_eq!(options.max_concurrent_streams, 100);
        assert_eq!(options.initial_max_data, 10_000_000);
    }

    #[test]
    fn test_network_conditions() {
        let conditions = NetworkConditions {
            bandwidth_estimate: 10_000_000,
            rtt: std::time::Duration::from_millis(50),
            packet_loss_rate: 0.01,
            jitter: std::time::Duration::from_millis(5),
            congestion_level: CongestionLevel::Low,
        };

        assert_eq!(conditions.bandwidth_estimate, 10_000_000);
        assert!(matches!(conditions.congestion_level, CongestionLevel::Low));
    }

    #[test]
    fn test_protocol_message_creation() {
        let message = ProtocolMessage {
            message_type: MessageType::SessionStart,
            payload: vec![1, 2, 3],
            sequence_number: 42,
            timestamp: std::time::SystemTime::now(),
            session_id: uuid::Uuid::new_v4(),
        };

        assert_eq!(message.sequence_number, 42);
        assert_eq!(message.payload.len(), 3);
        assert!(matches!(message.message_type, MessageType::SessionStart));
    }
}