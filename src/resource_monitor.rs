use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::collections::HashMap;
use std::fs;
use std::io;
use async_trait::async_trait;
use tokio::time::interval;
use crate::errors::ResourceError;

/// System resource monitor trait for monitoring storage, file integrity, and system resources
#[async_trait]
pub trait SystemResourceMonitor {
    /// Check available storage space
    async fn check_storage_space(&self, path: &Path, required_bytes: u64) -> Result<StorageInfo, ResourceError>;
    
    /// Monitor file integrity during transmission
    async fn monitor_file_integrity(&self, file_path: &Path) -> Result<FileIntegrityInfo, ResourceError>;
    
    /// Monitor system resource usage (CPU, memory)
    async fn monitor_system_resources(&self) -> Result<SystemResourceInfo, ResourceError>;
    
    /// Start continuous monitoring
    async fn start_monitoring(&mut self, config: MonitoringConfig) -> Result<(), ResourceError>;
    
    /// Stop monitoring
    async fn stop_monitoring(&mut self) -> Result<(), ResourceError>;
    
    /// Get current resource status
    async fn get_resource_status(&self) -> Result<ResourceStatus, ResourceError>;
    
    /// Adjust system parameters based on resource availability
    async fn adjust_parameters(&self, current_resources: &SystemResourceInfo) -> Result<ParameterAdjustments, ResourceError>;
}

/// Default implementation of system resource monitor
pub struct DefaultSystemResourceMonitor {
    monitoring_active: bool,
    file_checksums: HashMap<PathBuf, String>,
    file_modification_times: HashMap<PathBuf, SystemTime>,
    resource_history: Vec<SystemResourceSnapshot>,
    config: Option<MonitoringConfig>,
    monitoring_handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    pub check_interval: Duration,
    pub storage_warning_threshold: u64, // bytes
    pub storage_critical_threshold: u64, // bytes
    pub memory_warning_threshold: u64, // MB
    pub memory_critical_threshold: u64, // MB
    pub cpu_warning_threshold: f64, // percentage
    pub cpu_critical_threshold: f64, // percentage
    pub file_integrity_check_interval: Duration,
    pub max_history_entries: usize,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            storage_warning_threshold: 1_000_000_000, // 1GB
            storage_critical_threshold: 500_000_000,  // 500MB
            memory_warning_threshold: 1024, // 1GB
            memory_critical_threshold: 512,  // 512MB
            cpu_warning_threshold: 80.0,
            cpu_critical_threshold: 95.0,
            file_integrity_check_interval: Duration::from_secs(60),
            max_history_entries: 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageInfo {
    pub total_space: u64,
    pub available_space: u64,
    pub used_space: u64,
    pub usage_percentage: f64,
    pub is_sufficient: bool,
    pub warning_level: StorageWarningLevel,
}

#[derive(Debug, Clone)]
pub enum StorageWarningLevel {
    Normal,
    Warning,
    Critical,
    Insufficient,
}

#[derive(Debug, Clone)]
pub struct FileIntegrityInfo {
    pub file_path: PathBuf,
    pub current_checksum: String,
    pub last_known_checksum: Option<String>,
    pub modification_time: SystemTime,
    pub last_check_time: SystemTime,
    pub integrity_status: FileIntegrityStatus,
    pub file_size: u64,
}

#[derive(Debug, Clone)]
pub enum FileIntegrityStatus {
    Intact,
    Modified,
    Corrupted,
    Missing,
    AccessDenied,
}

#[derive(Debug, Clone)]
pub struct SystemResourceInfo {
    pub memory_usage: MemoryUsage,
    pub cpu_usage: CpuUsage,
    pub disk_io: DiskIoStats,
    pub network_io: NetworkIoStats,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone)]
pub struct MemoryUsage {
    pub total_memory: u64, // bytes
    pub used_memory: u64,  // bytes
    pub available_memory: u64, // bytes
    pub usage_percentage: f64,
    pub warning_level: ResourceWarningLevel,
}

#[derive(Debug, Clone)]
pub struct CpuUsage {
    pub usage_percentage: f64,
    pub load_average: f64,
    pub warning_level: ResourceWarningLevel,
}

#[derive(Debug, Clone)]
pub struct DiskIoStats {
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
    pub io_utilization: f64,
}

#[derive(Debug, Clone)]
pub struct NetworkIoStats {
    pub bytes_sent_per_sec: u64,
    pub bytes_received_per_sec: u64,
    pub network_utilization: f64,
}

#[derive(Debug, Clone)]
pub enum ResourceWarningLevel {
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct ResourceStatus {
    pub storage_status: HashMap<PathBuf, StorageInfo>,
    pub file_integrity_status: HashMap<PathBuf, FileIntegrityInfo>,
    pub system_resources: SystemResourceInfo,
    pub overall_health: SystemHealth,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SystemHealth {
    Healthy,
    Warning,
    Critical,
    Degraded,
}

#[derive(Debug, Clone)]
pub struct ParameterAdjustments {
    pub reduce_quality: bool,
    pub reduce_concurrent_streams: bool,
    pub increase_buffer_size: bool,
    pub reduce_segment_size: bool,
    pub pause_transmission: bool,
    pub adjustments: Vec<ParameterAdjustment>,
}

#[derive(Debug, Clone)]
pub struct ParameterAdjustment {
    pub parameter: String,
    pub old_value: String,
    pub new_value: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct SystemResourceSnapshot {
    pub timestamp: SystemTime,
    pub resources: SystemResourceInfo,
}

impl DefaultSystemResourceMonitor {
    pub fn new() -> Self {
        Self {
            monitoring_active: false,
            file_checksums: HashMap::new(),
            file_modification_times: HashMap::new(),
            resource_history: Vec::new(),
            config: None,
            monitoring_handle: None,
        }
    }

    pub fn with_config(config: MonitoringConfig) -> Self {
        Self {
            monitoring_active: false,
            file_checksums: HashMap::new(),
            file_modification_times: HashMap::new(),
            resource_history: Vec::new(),
            config: Some(config),
            monitoring_handle: None,
        }
    }

    /// Calculate file checksum using SHA-256
    async fn calculate_file_checksum(&self, file_path: &Path) -> Result<String, ResourceError> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let content = tokio::fs::read(file_path).await.map_err(|_e| {
            ResourceError::FileIntegrityFailed {
                file_path: file_path.to_string_lossy().to_string(),
            }
        })?;

        // Simple hash for demonstration - in production, use proper SHA-256
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Ok(format!("{:x}", hasher.finish()))
    }

    /// Get disk usage information for a path
    async fn get_disk_usage(&self, path: &Path) -> Result<(u64, u64, u64), ResourceError> {
        // This is a simplified implementation
        // In production, you would use platform-specific APIs like statvfs on Unix or GetDiskFreeSpace on Windows
        
        let _metadata = fs::metadata(path).map_err(|_| {
            ResourceError::MonitoringFailed {
                reason: format!("Cannot access path: {}", path.display()),
            }
        })?;

        // Simulate disk usage - in real implementation, use system calls
        let total_space = 100_000_000_000u64; // 100GB simulated
        let used_space = 50_000_000_000u64;   // 50GB simulated
        let available_space = total_space - used_space;

        Ok((total_space, available_space, used_space))
    }

    /// Get current memory usage
    fn get_memory_usage(&self) -> Result<MemoryUsage, ResourceError> {
        // This is a simplified implementation
        // In production, you would use platform-specific APIs or libraries like sysinfo
        
        // Simulate memory usage
        let total_memory = 8_000_000_000u64; // 8GB simulated
        let used_memory = 4_000_000_000u64;  // 4GB simulated
        let available_memory = total_memory - used_memory;
        let usage_percentage = (used_memory as f64 / total_memory as f64) * 100.0;

        let warning_level = if let Some(config) = &self.config {
            let used_mb = used_memory / 1_000_000;
            if used_mb > config.memory_critical_threshold {
                ResourceWarningLevel::Critical
            } else if used_mb > config.memory_warning_threshold {
                ResourceWarningLevel::Warning
            } else {
                ResourceWarningLevel::Normal
            }
        } else {
            ResourceWarningLevel::Normal
        };

        Ok(MemoryUsage {
            total_memory,
            used_memory,
            available_memory,
            usage_percentage,
            warning_level,
        })
    }

    /// Get current CPU usage
    fn get_cpu_usage(&self) -> Result<CpuUsage, ResourceError> {
        // This is a simplified implementation
        // In production, you would use platform-specific APIs or libraries like sysinfo
        
        // Simulate CPU usage
        let usage_percentage = 45.0; // 45% simulated
        let load_average = 2.5;

        let warning_level = if let Some(config) = &self.config {
            if usage_percentage > config.cpu_critical_threshold {
                ResourceWarningLevel::Critical
            } else if usage_percentage > config.cpu_warning_threshold {
                ResourceWarningLevel::Warning
            } else {
                ResourceWarningLevel::Normal
            }
        } else {
            ResourceWarningLevel::Normal
        };

        Ok(CpuUsage {
            usage_percentage,
            load_average,
            warning_level,
        })
    }

    /// Get disk I/O statistics
    fn get_disk_io_stats(&self) -> Result<DiskIoStats, ResourceError> {
        // Simulate disk I/O stats
        Ok(DiskIoStats {
            read_bytes_per_sec: 10_000_000,  // 10MB/s
            write_bytes_per_sec: 5_000_000,  // 5MB/s
            io_utilization: 25.0,            // 25%
        })
    }

    /// Get network I/O statistics
    fn get_network_io_stats(&self) -> Result<NetworkIoStats, ResourceError> {
        // Simulate network I/O stats
        Ok(NetworkIoStats {
            bytes_sent_per_sec: 1_000_000,    // 1MB/s
            bytes_received_per_sec: 500_000,  // 500KB/s
            network_utilization: 10.0,        // 10%
        })
    }

    /// Add resource snapshot to history
    fn add_resource_snapshot(&mut self, resources: SystemResourceInfo) {
        let snapshot = SystemResourceSnapshot {
            timestamp: SystemTime::now(),
            resources,
        };

        self.resource_history.push(snapshot);

        // Limit history size
        if let Some(config) = &self.config {
            if self.resource_history.len() > config.max_history_entries {
                self.resource_history.remove(0);
            }
        }
    }

    /// Generate recommendations based on current resource status
    fn generate_recommendations(&self, status: &ResourceStatus) -> Vec<String> {
        let mut recommendations = Vec::new();

        // Storage recommendations
        for (path, storage) in &status.storage_status {
            match storage.warning_level {
                StorageWarningLevel::Critical => {
                    recommendations.push(format!(
                        "Critical: Only {:.1}GB available on {}. Consider freeing up space or changing storage location.",
                        storage.available_space as f64 / 1_000_000_000.0,
                        path.display()
                    ));
                }
                StorageWarningLevel::Warning => {
                    recommendations.push(format!(
                        "Warning: Low disk space on {}. {:.1}GB available.",
                        path.display(),
                        storage.available_space as f64 / 1_000_000_000.0
                    ));
                }
                StorageWarningLevel::Insufficient => {
                    recommendations.push(format!(
                        "Error: Insufficient storage space on {}. Operation cannot continue.",
                        path.display()
                    ));
                }
                _ => {}
            }
        }

        // Memory recommendations
        match status.system_resources.memory_usage.warning_level {
            ResourceWarningLevel::Critical => {
                recommendations.push(format!(
                    "Critical: High memory usage ({:.1}%). Consider reducing concurrent operations or increasing system memory.",
                    status.system_resources.memory_usage.usage_percentage
                ));
            }
            ResourceWarningLevel::Warning => {
                recommendations.push(format!(
                    "Warning: Elevated memory usage ({:.1}%). Monitor for potential issues.",
                    status.system_resources.memory_usage.usage_percentage
                ));
            }
            _ => {}
        }

        // CPU recommendations
        match status.system_resources.cpu_usage.warning_level {
            ResourceWarningLevel::Critical => {
                recommendations.push(format!(
                    "Critical: High CPU usage ({:.1}%). Consider reducing processing load or upgrading hardware.",
                    status.system_resources.cpu_usage.usage_percentage
                ));
            }
            ResourceWarningLevel::Warning => {
                recommendations.push(format!(
                    "Warning: Elevated CPU usage ({:.1}%). Monitor system performance.",
                    status.system_resources.cpu_usage.usage_percentage
                ));
            }
            _ => {}
        }

        // File integrity recommendations
        for (path, integrity) in &status.file_integrity_status {
            match integrity.integrity_status {
                FileIntegrityStatus::Modified => {
                    recommendations.push(format!(
                        "Warning: File {} has been modified during transmission. Consider pausing and verifying file integrity.",
                        path.display()
                    ));
                }
                FileIntegrityStatus::Corrupted => {
                    recommendations.push(format!(
                        "Critical: File {} appears to be corrupted. Stop transmission and investigate.",
                        path.display()
                    ));
                }
                FileIntegrityStatus::Missing => {
                    recommendations.push(format!(
                        "Critical: File {} is missing. Transmission cannot continue.",
                        path.display()
                    ));
                }
                _ => {}
            }
        }

        recommendations
    }

    /// Determine overall system health
    fn determine_system_health(&self, status: &ResourceStatus) -> SystemHealth {
        let mut has_critical = false;
        let mut has_warning = false;

        // Check storage health
        for storage in status.storage_status.values() {
            match storage.warning_level {
                StorageWarningLevel::Critical | StorageWarningLevel::Insufficient => {
                    has_critical = true;
                }
                StorageWarningLevel::Warning => {
                    has_warning = true;
                }
                _ => {}
            }
        }

        // Check system resource health
        match status.system_resources.memory_usage.warning_level {
            ResourceWarningLevel::Critical => has_critical = true,
            ResourceWarningLevel::Warning => has_warning = true,
            _ => {}
        }

        match status.system_resources.cpu_usage.warning_level {
            ResourceWarningLevel::Critical => has_critical = true,
            ResourceWarningLevel::Warning => has_warning = true,
            _ => {}
        }

        // Check file integrity health
        for integrity in status.file_integrity_status.values() {
            match integrity.integrity_status {
                FileIntegrityStatus::Corrupted | FileIntegrityStatus::Missing => {
                    has_critical = true;
                }
                FileIntegrityStatus::Modified => {
                    has_warning = true;
                }
                _ => {}
            }
        }

        if has_critical {
            SystemHealth::Critical
        } else if has_warning {
            SystemHealth::Warning
        } else {
            SystemHealth::Healthy
        }
    }
}

#[async_trait]
impl SystemResourceMonitor for DefaultSystemResourceMonitor {
    async fn check_storage_space(&self, path: &Path, required_bytes: u64) -> Result<StorageInfo, ResourceError> {
        let (total_space, available_space, used_space) = self.get_disk_usage(path).await?;
        
        let usage_percentage = (used_space as f64 / total_space as f64) * 100.0;
        let is_sufficient = available_space >= required_bytes;

        let warning_level = if !is_sufficient {
            StorageWarningLevel::Insufficient
        } else if let Some(config) = &self.config {
            if available_space < config.storage_critical_threshold {
                StorageWarningLevel::Critical
            } else if available_space < config.storage_warning_threshold {
                StorageWarningLevel::Warning
            } else {
                StorageWarningLevel::Normal
            }
        } else {
            StorageWarningLevel::Normal
        };

        Ok(StorageInfo {
            total_space,
            available_space,
            used_space,
            usage_percentage,
            is_sufficient,
            warning_level,
        })
    }

    async fn monitor_file_integrity(&self, file_path: &Path) -> Result<FileIntegrityInfo, ResourceError> {
        let current_checksum = self.calculate_file_checksum(file_path).await?;
        
        let metadata = fs::metadata(file_path).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ResourceError::FileIntegrityFailed {
                file_path: file_path.to_string_lossy().to_string(),
            },
            io::ErrorKind::PermissionDenied => ResourceError::FileIntegrityFailed {
                file_path: file_path.to_string_lossy().to_string(),
            },
            _ => ResourceError::FileIntegrityFailed {
                file_path: file_path.to_string_lossy().to_string(),
            },
        })?;

        let modification_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let file_size = metadata.len();

        let last_known_checksum = self.file_checksums.get(file_path).cloned();
        let last_modification_time = self.file_modification_times.get(file_path).copied();

        let integrity_status = if let Some(last_checksum) = &last_known_checksum {
            if current_checksum != *last_checksum {
                if let Some(last_mod_time) = last_modification_time {
                    if modification_time > last_mod_time {
                        FileIntegrityStatus::Modified
                    } else {
                        FileIntegrityStatus::Corrupted
                    }
                } else {
                    FileIntegrityStatus::Modified
                }
            } else {
                FileIntegrityStatus::Intact
            }
        } else {
            FileIntegrityStatus::Intact // First check
        };

        Ok(FileIntegrityInfo {
            file_path: file_path.to_path_buf(),
            current_checksum,
            last_known_checksum,
            modification_time,
            last_check_time: SystemTime::now(),
            integrity_status,
            file_size,
        })
    }

    async fn monitor_system_resources(&self) -> Result<SystemResourceInfo, ResourceError> {
        let memory_usage = self.get_memory_usage()?;
        let cpu_usage = self.get_cpu_usage()?;
        let disk_io = self.get_disk_io_stats()?;
        let network_io = self.get_network_io_stats()?;

        Ok(SystemResourceInfo {
            memory_usage,
            cpu_usage,
            disk_io,
            network_io,
            timestamp: SystemTime::now(),
        })
    }

    async fn start_monitoring(&mut self, config: MonitoringConfig) -> Result<(), ResourceError> {
        if self.monitoring_active {
            return Ok(()); // Already monitoring
        }

        self.config = Some(config.clone());
        self.monitoring_active = true;

        log::info!("Starting system resource monitoring with interval {:?}", config.check_interval);

        // Start monitoring task
        let monitoring_handle = {
            let check_interval = config.check_interval;
            let file_integrity_interval = config.file_integrity_check_interval;
            
            tokio::spawn(async move {
                let mut check_timer = interval(check_interval);
                let mut integrity_timer = interval(file_integrity_interval);

                loop {
                    tokio::select! {
                        _ = check_timer.tick() => {
                            // Perform system resource check
                            log::debug!("Performing system resource check");
                            // In a real implementation, this would update the monitor state
                        }
                        _ = integrity_timer.tick() => {
                            // Perform file integrity check
                            log::debug!("Performing file integrity check");
                            // In a real implementation, this would check all monitored files
                        }
                    }
                }
            })
        };

        self.monitoring_handle = Some(monitoring_handle);

        Ok(())
    }

    async fn stop_monitoring(&mut self) -> Result<(), ResourceError> {
        if !self.monitoring_active {
            return Ok(()); // Not monitoring
        }

        self.monitoring_active = false;

        if let Some(handle) = self.monitoring_handle.take() {
            handle.abort();
            log::info!("System resource monitoring stopped");
        }

        Ok(())
    }

    async fn get_resource_status(&self) -> Result<ResourceStatus, ResourceError> {
        let system_resources = self.monitor_system_resources().await?;
        
        // Check storage for common paths
        let mut storage_status = HashMap::new();
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let storage_info = self.check_storage_space(&current_dir, 1_000_000_000).await?; // Check for 1GB
        storage_status.insert(current_dir, storage_info);

        // Check file integrity for monitored files
        let mut file_integrity_status = HashMap::new();
        for file_path in self.file_checksums.keys() {
            if let Ok(integrity_info) = self.monitor_file_integrity(file_path).await {
                file_integrity_status.insert(file_path.clone(), integrity_info);
            }
        }

        let status = ResourceStatus {
            storage_status,
            file_integrity_status,
            system_resources,
            overall_health: SystemHealth::Healthy, // Will be updated below
            recommendations: Vec::new(), // Will be updated below
        };

        let overall_health = self.determine_system_health(&status);
        let recommendations = self.generate_recommendations(&status);

        Ok(ResourceStatus {
            overall_health,
            recommendations,
            ..status
        })
    }

    async fn adjust_parameters(&self, current_resources: &SystemResourceInfo) -> Result<ParameterAdjustments, ResourceError> {
        let mut adjustments = Vec::new();
        let mut reduce_quality = false;
        let mut reduce_concurrent_streams = false;
        let mut increase_buffer_size = false;
        let mut reduce_segment_size = false;
        let pause_transmission = false;

        // Memory-based adjustments
        match current_resources.memory_usage.warning_level {
            ResourceWarningLevel::Critical => {
                reduce_concurrent_streams = true;
                reduce_segment_size = true;
                adjustments.push(ParameterAdjustment {
                    parameter: "concurrent_streams".to_string(),
                    old_value: "8".to_string(),
                    new_value: "4".to_string(),
                    reason: "Critical memory usage detected".to_string(),
                });
                adjustments.push(ParameterAdjustment {
                    parameter: "segment_size".to_string(),
                    old_value: "1MB".to_string(),
                    new_value: "512KB".to_string(),
                    reason: "Reduce memory footprint".to_string(),
                });
            }
            ResourceWarningLevel::Warning => {
                increase_buffer_size = true;
                adjustments.push(ParameterAdjustment {
                    parameter: "buffer_size".to_string(),
                    old_value: "64KB".to_string(),
                    new_value: "32KB".to_string(),
                    reason: "Elevated memory usage".to_string(),
                });
            }
            _ => {}
        }

        // CPU-based adjustments
        match current_resources.cpu_usage.warning_level {
            ResourceWarningLevel::Critical => {
                reduce_quality = true;
                reduce_concurrent_streams = true;
                adjustments.push(ParameterAdjustment {
                    parameter: "video_quality".to_string(),
                    old_value: "high".to_string(),
                    new_value: "medium".to_string(),
                    reason: "Critical CPU usage detected".to_string(),
                });
            }
            ResourceWarningLevel::Warning => {
                adjustments.push(ParameterAdjustment {
                    parameter: "processing_threads".to_string(),
                    old_value: "auto".to_string(),
                    new_value: "2".to_string(),
                    reason: "Limit CPU usage".to_string(),
                });
            }
            _ => {}
        }

        // Disk I/O adjustments
        if current_resources.disk_io.io_utilization > 90.0 {
            increase_buffer_size = true;
            adjustments.push(ParameterAdjustment {
                parameter: "io_buffer_size".to_string(),
                old_value: "64KB".to_string(),
                new_value: "128KB".to_string(),
                reason: "High disk I/O utilization".to_string(),
            });
        }

        Ok(ParameterAdjustments {
            reduce_quality,
            reduce_concurrent_streams,
            increase_buffer_size,
            reduce_segment_size,
            pause_transmission,
            adjustments,
        })
    }
}

impl DefaultSystemResourceMonitor {
    /// Add a file to integrity monitoring
    pub async fn add_file_to_monitoring(&mut self, file_path: &Path) -> Result<(), ResourceError> {
        let checksum = self.calculate_file_checksum(file_path).await?;
        let metadata = fs::metadata(file_path).map_err(|_| {
            ResourceError::FileIntegrityFailed {
                file_path: file_path.to_string_lossy().to_string(),
            }
        })?;
        
        let modification_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        
        self.file_checksums.insert(file_path.to_path_buf(), checksum);
        self.file_modification_times.insert(file_path.to_path_buf(), modification_time);
        
        log::info!("Added file to integrity monitoring: {}", file_path.display());
        Ok(())
    }

    /// Remove a file from integrity monitoring
    pub fn remove_file_from_monitoring(&mut self, file_path: &Path) {
        self.file_checksums.remove(file_path);
        self.file_modification_times.remove(file_path);
        log::info!("Removed file from integrity monitoring: {}", file_path.display());
    }

    /// Get resource usage history
    pub fn get_resource_history(&self) -> &[SystemResourceSnapshot] {
        &self.resource_history
    }

    /// Clear resource history
    pub fn clear_resource_history(&mut self) {
        self.resource_history.clear();
    }
}