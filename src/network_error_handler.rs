use std::time::{Duration, SystemTime};
use std::net::SocketAddr;
use std::collections::HashMap;
use async_trait::async_trait;
use tokio::time::sleep;
use uuid::Uuid;
use crate::errors::{TransportError, NetworkError};
use crate::types::{
    QUICConnection, QUICOptions, RecoveryState,
    ProtocolVersion, ProtocolMessage, MessageType, StatusCode
};

/// Network error handler trait for managing connection failures and recovery
#[async_trait]
pub trait NetworkErrorHandler {
    /// Handle connection failures with retry logic
    async fn handle_connection_failure(
        &self,
        error: &TransportError,
        server_address: SocketAddr,
        options: QUICOptions,
        recovery_state: &mut RecoveryState,
    ) -> Result<QUICConnection, NetworkError>;
    
    /// Parse and handle server error responses
    async fn handle_server_error(
        &self,
        error_code: u16,
        error_message: &str,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError>;
    
    /// Negotiate protocol version with server
    async fn negotiate_protocol_version(
        &self,
        connection: &mut QUICConnection,
        client_version: ProtocolVersion,
    ) -> Result<ProtocolVersion, NetworkError>;
    
    /// Monitor network conditions and adapt parameters
    async fn monitor_and_adapt(
        &self,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError>;
    
    /// Handle protocol-level errors
    async fn handle_protocol_error(
        &self,
        error_code: u16,
        description: &str,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError>;
}

/// Default implementation of network error handler
pub struct DefaultNetworkErrorHandler {
    retry_config: RetryConfig,
    protocol_config: ProtocolConfig,
    error_statistics: tokio::sync::Mutex<ErrorStatistics>,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolConfig {
    pub supported_versions: Vec<ProtocolVersion>,
    pub negotiation_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub max_protocol_errors: u32,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            supported_versions: vec![
                ProtocolVersion { major: 1, minor: 0, patch: 0 },
                ProtocolVersion { major: 1, minor: 1, patch: 0 },
            ],
            negotiation_timeout: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(30),
            max_protocol_errors: 10,
        }
    }
}

#[derive(Debug, Default)]
pub struct ErrorStatistics {
    pub connection_failures: u32,
    pub server_errors: HashMap<u16, u32>,
    pub protocol_errors: u32,
    pub recovery_attempts: u32,
    pub successful_recoveries: u32,
    pub last_error_time: Option<SystemTime>,
}

impl DefaultNetworkErrorHandler {
    pub fn new() -> Self {
        Self {
            retry_config: RetryConfig::default(),
            protocol_config: ProtocolConfig::default(),
            error_statistics: tokio::sync::Mutex::new(ErrorStatistics::default()),
        }
    }

    pub fn with_config(retry_config: RetryConfig, protocol_config: ProtocolConfig) -> Self {
        Self {
            retry_config,
            protocol_config,
            error_statistics: tokio::sync::Mutex::new(ErrorStatistics::default()),
        }
    }

    /// Calculate backoff delay with jitter
    fn calculate_backoff_delay(&self, attempt: u32) -> Duration {
        let base_delay_ms = self.retry_config.base_delay.as_millis() as f64;
        let delay_ms = base_delay_ms * self.retry_config.backoff_multiplier.powi(attempt as i32);
        
        // Apply maximum delay limit
        let delay_ms = delay_ms.min(self.retry_config.max_delay.as_millis() as f64);
        
        // Add jitter to prevent thundering herd
        let jitter = delay_ms * self.retry_config.jitter_factor * (rand::random::<f64>() - 0.5);
        let final_delay_ms = (delay_ms + jitter).max(0.0) as u64;
        
        Duration::from_millis(final_delay_ms)
    }

    /// Determine if error is retryable
    fn is_retryable_error(&self, error: &TransportError) -> bool {
        match error {
            TransportError::ConnectionFailed { .. } => true,
            TransportError::Timeout { .. } => true,
            TransportError::NetworkUnreachable { .. } => true,
            TransportError::DnsResolutionFailed { .. } => true,
            TransportError::ServerError { code, .. } => {
                // Retry on server errors that might be temporary
                matches!(*code, 500..=599 | 1002 | 1003) // Internal errors, bandwidth, storage
            }
            TransportError::RateLimitExceeded { .. } => true,
            TransportError::ConnectionLost { .. } => true,
            _ => false,
        }
    }

    /// Parse server error response
    fn parse_server_error(&self, error_code: u16, error_message: &str) -> ServerErrorInfo {
        let error_type = match error_code {
            400..=499 => ServerErrorType::ClientError,
            500..=599 => ServerErrorType::ServerError,
            1001 => ServerErrorType::UnsupportedFormat,
            1002 => ServerErrorType::InsufficientBandwidth,
            1003 => ServerErrorType::StorageFull,
            1004 => ServerErrorType::AuthenticationFailed,
            1005 => ServerErrorType::SegmentCorrupted,
            _ => ServerErrorType::Unknown,
        };

        let recovery_action = match error_code {
            400 => RecoveryAction::None, // Bad request - don't retry
            401 => RecoveryAction::Reauthenticate,
            404 => RecoveryAction::None, // Not found - don't retry
            429 => RecoveryAction::BackoffAndRetry,
            500..=503 => RecoveryAction::RetryWithBackoff,
            1001 => RecoveryAction::None, // Unsupported format - don't retry
            1002 => RecoveryAction::ReduceQuality,
            1003 => RecoveryAction::WaitAndRetry,
            1004 => RecoveryAction::Reauthenticate,
            1005 => RecoveryAction::ResendSegment,
            _ => RecoveryAction::RetryWithBackoff,
        };

        let is_retryable = matches!(recovery_action, 
            RecoveryAction::RetryWithBackoff | 
            RecoveryAction::BackoffAndRetry | 
            RecoveryAction::WaitAndRetry |
            RecoveryAction::ResendSegment
        );

        ServerErrorInfo {
            error_type,
            recovery_action,
            message: error_message.to_string(),
            is_retryable,
        }
    }

    /// Send protocol message with error handling
    async fn send_protocol_message_safe(
        &self,
        _connection: &mut QUICConnection,
        message: ProtocolMessage,
    ) -> Result<(), NetworkError> {
        // Implementation would use the transport layer to send the message
        // For now, we'll simulate the operation
        log::debug!("Sending protocol message: {:?}", message.message_type);
        
        // Simulate potential network errors
        if rand::random::<f64>() < 0.1 { // 10% chance of failure for testing
            return Err(NetworkError::ProtocolNegotiationFailed {
                reason: "Simulated network failure".to_string(),
            });
        }
        
        Ok(())
    }

    /// Wait for protocol response with timeout
    async fn wait_for_protocol_response(
        &self,
        _connection: &mut QUICConnection,
        timeout: Duration,
    ) -> Result<ProtocolMessage, NetworkError> {
        // Implementation would wait for actual response from server
        // For now, we'll simulate a successful response
        tokio::time::timeout(timeout, async {
            // Simulate response delay
            sleep(Duration::from_millis(100)).await;
            
            Ok(ProtocolMessage {
                message_type: MessageType::StatusResponse,
                payload: vec![200, 0], // Success status
                sequence_number: 1,
                timestamp: SystemTime::now(),
                session_id: Uuid::new_v4(),
            })
        })
        .await
        .map_err(|_| NetworkError::ProtocolNegotiationFailed {
            reason: "Protocol negotiation timeout".to_string(),
        })?
    }
}

#[async_trait]
impl NetworkErrorHandler for DefaultNetworkErrorHandler {
    async fn handle_connection_failure(
        &self,
        error: &TransportError,
        server_address: SocketAddr,
        options: QUICOptions,
        recovery_state: &mut RecoveryState,
    ) -> Result<QUICConnection, NetworkError> {
        let mut stats = self.error_statistics.lock().await;
        stats.connection_failures += 1;
        stats.last_error_time = Some(SystemTime::now());
        drop(stats);

        log::warn!("Connection failure: {} (attempt {})", error, recovery_state.retry_count + 1);

        // Check if error is retryable
        if !self.is_retryable_error(error) {
            return Err(NetworkError::RecoveryFailed {
                reason: format!("Non-retryable error: {}", error),
            });
        }

        // Check retry limits
        if recovery_state.retry_count >= self.retry_config.max_retries {
            return Err(NetworkError::RetryExhausted {
                attempts: recovery_state.retry_count,
            });
        }

        // Calculate backoff delay
        let backoff_delay = self.calculate_backoff_delay(recovery_state.retry_count);
        recovery_state.backoff_duration = backoff_delay;
        recovery_state.retry_count += 1;

        log::info!("Retrying connection in {:?} (attempt {})", 
                  backoff_delay, recovery_state.retry_count);

        // Apply backoff delay
        sleep(backoff_delay).await;

        // Update statistics
        let mut stats = self.error_statistics.lock().await;
        stats.recovery_attempts += 1;
        drop(stats);

        // Attempt to reconnect using the transport layer
        // This would typically use the QUICTransport trait
        // For now, we'll simulate the reconnection attempt
        match self.simulate_reconnection(server_address, options).await {
            Ok(connection) => {
                let mut stats = self.error_statistics.lock().await;
                stats.successful_recoveries += 1;
                drop(stats);
                
                log::info!("Connection recovered successfully");
                recovery_state.retry_count = 0; // Reset on success
                Ok(connection)
            }
            Err(e) => {
                log::error!("Connection recovery failed: {}", e);
                Err(NetworkError::RecoveryFailed {
                    reason: e.to_string(),
                })
            }
        }
    }

    async fn handle_server_error(
        &self,
        error_code: u16,
        error_message: &str,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError> {
        let mut stats = self.error_statistics.lock().await;
        *stats.server_errors.entry(error_code).or_insert(0) += 1;
        drop(stats);

        let error_info = self.parse_server_error(error_code, error_message);
        
        log::warn!("Server error {}: {} (type: {:?}, action: {:?})", 
                  error_code, error_message, error_info.error_type, error_info.recovery_action);

        match error_info.recovery_action {
            RecoveryAction::None => {
                return Err(NetworkError::ResponseParsingFailed {
                    reason: format!("Unrecoverable server error: {}", error_message),
                });
            }
            RecoveryAction::Reauthenticate => {
                log::info!("Attempting reauthentication");
                // Send authentication request
                let auth_message = ProtocolMessage {
                    message_type: MessageType::SessionStart,
                    payload: b"reauthenticate".to_vec(),
                    sequence_number: connection.id.as_u128() as u64,
                    timestamp: SystemTime::now(),
                    session_id: connection.id,
                };
                self.send_protocol_message_safe(connection, auth_message).await?;
            }
            RecoveryAction::ReduceQuality => {
                log::info!("Reducing transmission quality due to bandwidth constraints");
                // This would typically adjust encoding parameters
                // For now, we'll just log the action
            }
            RecoveryAction::WaitAndRetry => {
                log::info!("Waiting before retry due to server overload");
                sleep(Duration::from_secs(5)).await;
            }
            RecoveryAction::ResendSegment => {
                log::info!("Preparing to resend corrupted segment");
                // This would typically trigger segment retransmission
            }
            RecoveryAction::RetryWithBackoff | RecoveryAction::BackoffAndRetry => {
                log::info!("Applying backoff before retry");
                let backoff = self.calculate_backoff_delay(1);
                sleep(backoff).await;
            }
        }

        Ok(())
    }

    async fn negotiate_protocol_version(
        &self,
        connection: &mut QUICConnection,
        client_version: ProtocolVersion,
    ) -> Result<ProtocolVersion, NetworkError> {
        log::info!("Starting protocol version negotiation with version {}.{}.{}", 
                  client_version.major, client_version.minor, client_version.patch);

        // Send version negotiation message
        let version_payload = format!("{}.{}.{}", 
                                    client_version.major, 
                                    client_version.minor, 
                                    client_version.patch).into_bytes();

        let negotiation_message = ProtocolMessage {
            message_type: MessageType::VersionNegotiation,
            payload: version_payload,
            sequence_number: 1,
            timestamp: SystemTime::now(),
            session_id: Uuid::new_v4(),
        };

        self.send_protocol_message_safe(connection, negotiation_message).await?;

        // Wait for server response
        let response = self.wait_for_protocol_response(
            connection, 
            self.protocol_config.negotiation_timeout
        ).await?;

        // Parse server response
        match response.message_type {
            MessageType::StatusResponse => {
                if response.payload.len() >= 2 {
                    let status_code = u16::from_be_bytes([response.payload[0], response.payload[1]]);
                    
                    if status_code == StatusCode::Success as u16 {
                        log::info!("Protocol version {}.{}.{} accepted by server", 
                                  client_version.major, client_version.minor, client_version.patch);
                        return Ok(client_version);
                    } else {
                        return Err(NetworkError::ProtocolNegotiationFailed {
                            reason: format!("Server rejected version with status: {}", status_code),
                        });
                    }
                }
            }
            MessageType::VersionNegotiation => {
                // Server proposed alternative version
                if response.payload.len() >= 3 {
                    let server_version = ProtocolVersion {
                        major: response.payload[0],
                        minor: response.payload[1],
                        patch: response.payload[2],
                    };
                    
                    // Check if we support the server's version
                    if self.protocol_config.supported_versions.iter()
                        .any(|v| v.is_compatible(&server_version)) {
                        log::info!("Accepting server's protocol version {}.{}.{}", 
                                  server_version.major, server_version.minor, server_version.patch);
                        return Ok(server_version);
                    } else {
                        return Err(NetworkError::ProtocolNegotiationFailed {
                            reason: format!("Incompatible protocol version: server {}.{}.{}", 
                                          server_version.major, server_version.minor, server_version.patch),
                        });
                    }
                }
            }
            _ => {
                return Err(NetworkError::ProtocolNegotiationFailed {
                    reason: "Unexpected response type during version negotiation".to_string(),
                });
            }
        }

        Err(NetworkError::ProtocolNegotiationFailed {
            reason: "Invalid server response format".to_string(),
        })
    }

    async fn monitor_and_adapt(
        &self,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError> {
        let stats = connection.stats.clone();
        
        // Monitor connection health
        if stats.packet_loss > 0.1 {
            log::warn!("High packet loss detected: {:.2}%", stats.packet_loss * 100.0);
            
            // Adapt transmission parameters
            // This would typically adjust QUIC transport parameters
            log::info!("Adapting to high packet loss conditions");
        }

        if stats.rtt > Duration::from_millis(500) {
            log::warn!("High latency detected: {:?}", stats.rtt);
            
            // Adjust for high latency
            log::info!("Adapting to high latency conditions");
        }

        // Check for network degradation
        let degradation_threshold = 0.05; // 5% packet loss threshold
        if stats.packet_loss > degradation_threshold {
            return Err(NetworkError::NetworkDegraded {
                condition: format!("Packet loss: {:.2}%", stats.packet_loss * 100.0),
            });
        }

        Ok(())
    }

    async fn handle_protocol_error(
        &self,
        error_code: u16,
        description: &str,
        connection: &mut QUICConnection,
    ) -> Result<(), NetworkError> {
        let mut stats = self.error_statistics.lock().await;
        stats.protocol_errors += 1;
        drop(stats);

        log::error!("Protocol error {}: {}", error_code, description);

        // Check if we've exceeded the maximum protocol errors
        let stats = self.error_statistics.lock().await;
        if stats.protocol_errors > self.protocol_config.max_protocol_errors {
            drop(stats);
            return Err(NetworkError::ProtocolNegotiationFailed {
                reason: "Too many protocol errors, connection unstable".to_string(),
            });
        }
        drop(stats);

        // Send error acknowledgment
        let error_response = ProtocolMessage {
            message_type: MessageType::ErrorReport,
            payload: format!("ACK:{}", error_code).into_bytes(),
            sequence_number: connection.id.as_u128() as u64,
            timestamp: SystemTime::now(),
            session_id: connection.id,
        };

        self.send_protocol_message_safe(connection, error_response).await?;

        Ok(())
    }
}

impl DefaultNetworkErrorHandler {
    /// Simulate reconnection attempt (placeholder for actual implementation)
    async fn simulate_reconnection(
        &self,
        _server_address: SocketAddr,
        _options: QUICOptions,
    ) -> Result<QUICConnection, TransportError> {
        // This would typically use the actual QUICTransport implementation
        // For now, we'll simulate a successful connection
        sleep(Duration::from_millis(100)).await;
        
        // Simulate occasional failures for testing
        if rand::random::<f64>() < 0.3 { // 30% failure rate
            return Err(TransportError::ConnectionFailed {
                reason: "Simulated connection failure".to_string(),
            });
        }


        
        // This is a placeholder implementation - in real code this would use actual QUIC transport
        Err(TransportError::ConnectionFailed {
            reason: "Placeholder implementation - actual reconnection not implemented".to_string(),
        })
    }

    /// Get error statistics for monitoring
    pub async fn get_error_statistics(&self) -> ErrorStatistics {
        self.error_statistics.lock().await.clone()
    }
}

#[derive(Debug, Clone)]
struct ServerErrorInfo {
    error_type: ServerErrorType,
    recovery_action: RecoveryAction,
    message: String,
    is_retryable: bool,
}

#[derive(Debug, Clone)]
enum ServerErrorType {
    ClientError,
    ServerError,
    UnsupportedFormat,
    InsufficientBandwidth,
    StorageFull,
    AuthenticationFailed,
    SegmentCorrupted,
    Unknown,
}

#[derive(Debug, Clone)]
enum RecoveryAction {
    None,
    Reauthenticate,
    ReduceQuality,
    WaitAndRetry,
    ResendSegment,
    RetryWithBackoff,
    BackoffAndRetry,
}

impl Clone for ErrorStatistics {
    fn clone(&self) -> Self {
        Self {
            connection_failures: self.connection_failures,
            server_errors: self.server_errors.clone(),
            protocol_errors: self.protocol_errors,
            recovery_attempts: self.recovery_attempts,
            successful_recoveries: self.successful_recoveries,
            last_error_time: self.last_error_time,
        }
    }
}