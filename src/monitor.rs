use std::time::{Duration, SystemTime};
use std::collections::VecDeque;
use crate::types::{Segment, PerformanceStats, PerformanceReport, ExportFormat, PerformanceAnomaly, AnomalyType, AnomalySeverity};
use crate::errors::ExportError;
use uuid::Uuid;

pub trait PerformanceMonitor {
    fn start_monitoring(&mut self);
    fn record_transmission(&mut self, segment: &Segment, latency: Duration);
    fn get_current_stats(&self) -> PerformanceStats;
    fn generate_report(&self) -> PerformanceReport;
    fn export_data(&self, format: ExportFormat) -> Result<String, ExportError>;
}

pub struct DefaultPerformanceMonitor {
    session_id: Uuid,
    start_time: Option<SystemTime>,
    transmission_records: VecDeque<TransmissionRecord>,
    total_bytes: u64,
    total_segments: u64,
    latency_samples: VecDeque<Duration>,
    max_samples: usize,
}

#[derive(Debug, Clone)]
struct TransmissionRecord {
    timestamp: SystemTime,
    segment_size: usize,
    latency: Duration,
}

impl DefaultPerformanceMonitor {
    pub fn new() -> Self {
        Self {
            session_id: Uuid::new_v4(),
            start_time: None,
            transmission_records: VecDeque::new(),
            total_bytes: 0,
            total_segments: 0,
            latency_samples: VecDeque::new(),
            max_samples: 1000, // Keep last 1000 samples
        }
    }
}

impl PerformanceMonitor for DefaultPerformanceMonitor {
    fn start_monitoring(&mut self) {
        self.start_time = Some(SystemTime::now());
        self.transmission_records.clear();
        self.total_bytes = 0;
        self.total_segments = 0;
        self.latency_samples.clear();
    }

    fn record_transmission(&mut self, segment: &Segment, latency: Duration) {
        let now = SystemTime::now();
        let segment_size = match segment {
            Segment::Video(v) => v.data.len(),
            Segment::Audio(a) => a.data.len(),
        };

        let record = TransmissionRecord {
            timestamp: now,
            segment_size,
            latency,
        };

        self.transmission_records.push_back(record);
        self.latency_samples.push_back(latency);
        self.total_bytes += segment_size as u64;
        self.total_segments += 1;

        // Keep only recent records
        if self.transmission_records.len() > self.max_samples {
            self.transmission_records.pop_front();
        }
        if self.latency_samples.len() > self.max_samples {
            self.latency_samples.pop_front();
        }
    }

    fn get_current_stats(&self) -> PerformanceStats {
        let current_time = SystemTime::now();
        let start_time = self.start_time.unwrap_or(current_time);
        
        let elapsed = current_time.duration_since(start_time).unwrap_or(Duration::from_secs(1));
        let elapsed_secs = elapsed.as_secs_f64();

        let average_upload_speed = if elapsed_secs > 0.0 {
            self.total_bytes as f64 / elapsed_secs
        } else {
            0.0
        };

        // Calculate current speed from recent samples (last 10 seconds)
        let recent_cutoff = current_time - Duration::from_secs(10);
        let recent_records: Vec<_> = self.transmission_records
            .iter()
            .filter(|r| r.timestamp >= recent_cutoff)
            .collect();

        let current_upload_speed = if !recent_records.is_empty() {
            let recent_bytes: usize = recent_records.iter().map(|r| r.segment_size).sum();
            let recent_duration = recent_records.last().unwrap().timestamp
                .duration_since(recent_records.first().unwrap().timestamp)
                .unwrap_or(Duration::from_secs(1));
            recent_bytes as f64 / recent_duration.as_secs_f64()
        } else {
            0.0
        };

        let average_latency = if !self.latency_samples.is_empty() {
            let total_latency: Duration = self.latency_samples.iter().sum();
            total_latency / self.latency_samples.len() as u32
        } else {
            Duration::from_millis(0)
        };

        let current_latency = self.latency_samples.back().copied().unwrap_or(Duration::from_millis(0));

        PerformanceStats {
            current_upload_speed,
            average_upload_speed,
            current_latency,
            average_latency,
            total_bytes_transmitted: self.total_bytes,
            segments_transmitted: self.total_segments,
        }
    }

    fn generate_report(&self) -> PerformanceReport {
        let stats = self.get_current_stats();
        let start_time = self.start_time.unwrap_or(SystemTime::now());
        let end_time = SystemTime::now();
        let total_duration = end_time.duration_since(start_time).unwrap_or(Duration::from_secs(0));

        // Find peak upload speed (simplified calculation since VecDeque doesn't have windows)
        let peak_upload_speed = if self.transmission_records.len() >= 10 {
            let mut max_speed: f64 = 0.0;
            for i in 0..=(self.transmission_records.len() - 10) {
                let window: Vec<_> = self.transmission_records.range(i..i+10).collect();
                let total_bytes: usize = window.iter().map(|r| r.segment_size).sum();
                let duration = window.last().unwrap().timestamp
                    .duration_since(window.first().unwrap().timestamp)
                    .unwrap_or(Duration::from_secs(1));
                let speed = total_bytes as f64 / duration.as_secs_f64();
                max_speed = max_speed.max(speed);
            }
            max_speed
        } else {
            stats.current_upload_speed
        };

        // Collect latency distribution
        let mut latency_distribution: Vec<Duration> = self.latency_samples.iter().copied().collect();
        latency_distribution.sort();

        // Detect anomalies (simplified)
        let anomalies = self.detect_anomalies();

        PerformanceReport {
            session_id: self.session_id,
            start_time,
            end_time,
            total_duration,
            stats,
            peak_upload_speed,
            latency_distribution,
            anomalies,
        }
    }

    fn export_data(&self, format: ExportFormat) -> Result<String, ExportError> {
        let report = self.generate_report();
        
        match format {
            ExportFormat::Json => {
                serde_json::to_string_pretty(&report)
                    .map_err(|_| ExportError::SerializationFailed)
            },
            ExportFormat::Csv => {
                let mut csv = String::new();
                csv.push_str("timestamp,segment_size,latency_ms\n");
                
                for record in &self.transmission_records {
                    let timestamp = record.timestamp
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or(Duration::from_secs(0))
                        .as_secs();
                    csv.push_str(&format!(
                        "{},{},{}\n",
                        timestamp,
                        record.segment_size,
                        record.latency.as_millis()
                    ));
                }
                
                Ok(csv)
            },
        }
    }
}

impl DefaultPerformanceMonitor {
    fn detect_anomalies(&self) -> Vec<PerformanceAnomaly> {
        let mut anomalies = Vec::new();
        
        // Simple anomaly detection based on latency spikes
        if let Some(avg_latency) = self.latency_samples.iter().copied().reduce(|a, b| a + b) {
            let avg_latency = avg_latency / self.latency_samples.len() as u32;
            let threshold = avg_latency * 3; // 3x average is considered anomalous
            
            for (i, &latency) in self.latency_samples.iter().enumerate() {
                if latency > threshold {
                    if let Some(record) = self.transmission_records.get(i) {
                        anomalies.push(PerformanceAnomaly {
                            timestamp: record.timestamp,
                            anomaly_type: AnomalyType::HighLatency,
                            description: format!("Latency spike: {}ms (avg: {}ms)", 
                                latency.as_millis(), avg_latency.as_millis()),
                            severity: if latency > threshold * 2 {
                                AnomalySeverity::High
                            } else {
                                AnomalySeverity::Medium
                            },
                        });
                    }
                }
            }
        }
        
        anomalies
    }
}