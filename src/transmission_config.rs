use std::time::Duration;

/// 传输模式配置
#[derive(Debug, Clone)]
pub enum TransmissionMode {
    /// 实时流媒体模式 - 低延迟优先
    RealTime {
        target_latency_ms: f64,
        frame_rate: f64,
    },
    /// 高吞吐模式 - 速度优先
    HighThroughput {
        max_segment_size: usize,
        parallel_streams: usize,
    },
    /// 平衡模式 - 延迟和吞吐平衡
    Balanced {
        latency_ms: f64,
        throughput_priority: f64, // 0.0-1.0
    },
}

/// 传输配置
#[derive(Debug, Clone)]
pub struct TransmissionConfig {
    pub mode: TransmissionMode,
    pub segment_size_range: (usize, usize), // (min, max)
    pub enable_adaptive_sizing: bool,
    pub bandwidth_limit_mbps: Option<f64>,
    pub enable_parallel_streams: bool,
    pub max_parallel_streams: usize,
}

impl Default for TransmissionConfig {
    fn default() -> Self {
        Self {
            mode: TransmissionMode::HighThroughput {
                max_segment_size: 1024 * 1024, // 1MB
                parallel_streams: 4,
            },
            segment_size_range: (8192, 1024 * 1024), // 8KB - 1MB
            enable_adaptive_sizing: true,
            bandwidth_limit_mbps: None,
            enable_parallel_streams: true,
            max_parallel_streams: 8,
        }
    }
}

impl TransmissionConfig {
    /// 创建实时流媒体配置
    pub fn real_time(target_latency_ms: f64, frame_rate: f64) -> Self {
        Self {
            mode: TransmissionMode::RealTime {
                target_latency_ms,
                frame_rate,
            },
            segment_size_range: (4096, 16384), // 4KB - 16KB
            enable_adaptive_sizing: false,
            bandwidth_limit_mbps: None,
            enable_parallel_streams: false,
            max_parallel_streams: 1,
        }
    }

    /// 创建高速文件传输配置
    pub fn high_throughput(max_segment_size: usize, parallel_streams: usize) -> Self {
        Self {
            mode: TransmissionMode::HighThroughput {
                max_segment_size,
                parallel_streams,
            },
            segment_size_range: (64 * 1024, max_segment_size), // 64KB - max
            enable_adaptive_sizing: true,
            bandwidth_limit_mbps: None,
            enable_parallel_streams: true,
            max_parallel_streams: parallel_streams,
        }
    }

    /// 创建平衡配置
    pub fn balanced(latency_ms: f64, throughput_priority: f64) -> Self {
        Self {
            mode: TransmissionMode::Balanced {
                latency_ms,
                throughput_priority,
            },
            segment_size_range: (16384, 256 * 1024), // 16KB - 256KB
            enable_adaptive_sizing: true,
            bandwidth_limit_mbps: None,
            enable_parallel_streams: throughput_priority > 0.5,
            max_parallel_streams: if throughput_priority > 0.7 { 4 } else { 2 },
        }
    }

    /// 计算最优分片大小
    pub fn calculate_optimal_segment_size(&self, bandwidth_mbps: f64) -> usize {
        match &self.mode {
            TransmissionMode::RealTime { target_latency_ms, .. } => {
                // 实时模式：基于延迟要求计算
                let max_bytes = (bandwidth_mbps * 1024.0 * 1024.0 / 8.0 * target_latency_ms / 1000.0) as usize;
                max_bytes.clamp(self.segment_size_range.0, self.segment_size_range.1)
            }
            TransmissionMode::HighThroughput { max_segment_size, .. } => {
                // 高吞吐模式：使用最大分片大小
                *max_segment_size
            }
            TransmissionMode::Balanced { latency_ms, throughput_priority } => {
                // 平衡模式：根据优先级调整
                let latency_based = (bandwidth_mbps * 1024.0 * 1024.0 / 8.0 * latency_ms / 1000.0) as usize;
                let throughput_based = self.segment_size_range.1;
                
                let weighted_size = (latency_based as f64 * (1.0 - throughput_priority) + 
                                   throughput_based as f64 * throughput_priority) as usize;
                
                weighted_size.clamp(self.segment_size_range.0, self.segment_size_range.1)
            }
        }
    }

    /// 计算传输延迟
    pub fn calculate_transmission_delay(&self, playback_rate: f64) -> Duration {
        match &self.mode {
            TransmissionMode::RealTime { target_latency_ms, .. } => {
                let delay_ms = (target_latency_ms / playback_rate).clamp(10.0, 100.0);
                Duration::from_millis(delay_ms as u64)
            }
            TransmissionMode::HighThroughput { .. } => {
                // 高吞吐模式：无延迟
                Duration::from_millis(0)
            }
            TransmissionMode::Balanced { latency_ms, throughput_priority } => {
                // 平衡模式：根据优先级调整延迟
                let delay_ms = (latency_ms * (1.0 - throughput_priority) / playback_rate).clamp(0.0, 50.0);
                Duration::from_millis(delay_ms as u64)
            }
        }
    }

    /// 是否应该使用并行传输
    pub fn should_use_parallel_transmission(&self) -> bool {
        self.enable_parallel_streams && match &self.mode {
            TransmissionMode::RealTime { .. } => false,
            TransmissionMode::HighThroughput { .. } => true,
            TransmissionMode::Balanced { throughput_priority, .. } => *throughput_priority > 0.5,
        }
    }
}

/// 传输性能指标
#[derive(Debug, Clone)]
pub struct TransmissionMetrics {
    pub start_time: std::time::Instant,
    pub bytes_sent: u64,
    pub segments_sent: u64,
    pub errors_count: u64,
    pub average_segment_size: f64,
    pub current_throughput_mbps: f64,
}

impl TransmissionMetrics {
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            bytes_sent: 0,
            segments_sent: 0,
            errors_count: 0,
            average_segment_size: 0.0,
            current_throughput_mbps: 0.0,
        }
    }

    pub fn record_segment(&mut self, segment_size: usize) {
        self.bytes_sent += segment_size as u64;
        self.segments_sent += 1;
        self.average_segment_size = self.bytes_sent as f64 / self.segments_sent as f64;
        self.update_throughput();
    }

    pub fn record_error(&mut self) {
        self.errors_count += 1;
    }

    fn update_throughput(&mut self) {
        let elapsed_secs = self.start_time.elapsed().as_secs_f64();
        if elapsed_secs > 0.0 {
            self.current_throughput_mbps = (self.bytes_sent as f64 / 1024.0 / 1024.0) / elapsed_secs;
        }
    }

    pub fn get_summary(&self) -> String {
        format!(
            "Transmission Summary:\n\
             - Duration: {:.2}s\n\
             - Bytes sent: {} ({:.2} MB)\n\
             - Segments sent: {}\n\
             - Average segment size: {:.1} KB\n\
             - Throughput: {:.2} MB/s\n\
             - Error rate: {:.2}%",
            self.start_time.elapsed().as_secs_f64(),
            self.bytes_sent,
            self.bytes_sent as f64 / 1024.0 / 1024.0,
            self.segments_sent,
            self.average_segment_size / 1024.0,
            self.current_throughput_mbps,
            if self.segments_sent > 0 {
                (self.errors_count as f64 / self.segments_sent as f64) * 100.0
            } else {
                0.0
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_throughput_config() {
        let config = TransmissionConfig::high_throughput(1024 * 1024, 4);
        assert!(matches!(config.mode, TransmissionMode::HighThroughput { .. }));
        assert_eq!(config.calculate_transmission_delay(1.0), Duration::from_millis(0));
        assert!(config.should_use_parallel_transmission());
    }

    #[test]
    fn test_real_time_config() {
        let config = TransmissionConfig::real_time(33.0, 30.0);
        assert!(matches!(config.mode, TransmissionMode::RealTime { .. }));
        assert_eq!(config.calculate_transmission_delay(1.0), Duration::from_millis(33));
        assert!(!config.should_use_parallel_transmission());
    }

    #[test]
    fn test_optimal_segment_size() {
        let config = TransmissionConfig::high_throughput(1024 * 1024, 4);
        let size = config.calculate_optimal_segment_size(100.0); // 100 Mbps
        assert_eq!(size, 1024 * 1024); // Should use max size
    }
}