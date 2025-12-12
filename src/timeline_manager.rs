use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error, debug};

use crate::ffmpeg_cli_parser::{FFmpegCliParser, TimelineData};
use crate::errors::{TimelineError, FFmpegError};

/// 时间轴管理器
/// 负责管理视频文件的时间轴数据，包括生成、缓存和加载
pub struct TimelineManager {
    /// FFmpeg 命令行解析器
    ffmpeg_parser: FFmpegCliParser,
    /// 缓存的时间轴数据
    cached_timelines: Arc<RwLock<HashMap<PathBuf, TimelineData>>>,
    /// 缓存大小限制 (MB)
    cache_limit_mb: usize,
    /// 当前缓存大小 (bytes)
    current_cache_size: Arc<RwLock<u64>>,
}

impl TimelineManager {
    /// 创建新的时间轴管理器
    pub fn new() -> Self {
        Self {
            ffmpeg_parser: FFmpegCliParser::new(),
            cached_timelines: Arc::new(RwLock::new(HashMap::new())),
            cache_limit_mb: 100, // 默认 100MB 缓存限制
            current_cache_size: Arc::new(RwLock::new(0)),
        }
    }

    /// 设置缓存大小限制
    pub fn set_cache_limit(&mut self, limit_mb: usize) {
        self.cache_limit_mb = limit_mb;
    }

    /// 获取或生成时间轴数据
    pub async fn get_timeline(&self, video_path: &Path) -> Result<TimelineData, TimelineError> {
        let video_path = video_path.to_path_buf();
        
        // 1. 检查内存缓存
        {
            let cache = self.cached_timelines.read().await;
            if let Some(timeline) = cache.get(&video_path) {
                debug!("Timeline found in memory cache: {:?}", video_path);
                return Ok(timeline.clone());
            }
        }

        // 2. 检查时间轴文件是否存在且有效
        let timeline_path = self.get_timeline_file_path(&video_path);
        if timeline_path.exists() && self.is_timeline_file_valid(&video_path, &timeline_path).await {
            info!("Loading existing timeline file: {:?}", timeline_path);
            match self.ffmpeg_parser.load_timeline_file(&video_path).await {
                Ok(timeline) => {
                    // 加载到缓存
                    self.add_to_cache(video_path, timeline.clone()).await?;
                    return Ok(timeline);
                }
                Err(e) => {
                    warn!("Failed to load timeline file {:?}: {}", timeline_path, e);
                    // 继续生成新的时间轴
                }
            }
        }

        // 3. 生成新的时间轴文件
        info!("Generating new timeline for video: {:?}", video_path);
        self.generate_timeline(&video_path).await
    }

    /// 生成新的时间轴文件
    async fn generate_timeline(&self, video_path: &Path) -> Result<TimelineData, TimelineError> {
        // 检查 FFmpeg 可用性
        match self.ffmpeg_parser.check_ffmpeg_availability().await {
            Ok(info) => {
                info!("Using FFmpeg: {}", info.version);
            }
            Err(FFmpegError::NotAvailable) => {
                return Err(TimelineError::FFmpeg(FFmpegError::NotAvailable));
            }
            Err(e) => {
                return Err(TimelineError::FFmpeg(e));
            }
        }

        // 解析视频文件
        let timeline_data = self.ffmpeg_parser.parse_video_file(video_path).await
            .map_err(TimelineError::FFmpeg)?;

        info!("Video parsing completed: {:.2}s duration, {} keyframes", 
              timeline_data.total_duration, timeline_data.keyframes.len());

        // 保存时间轴文件
        self.ffmpeg_parser.generate_timeline_file(video_path, &timeline_data).await
            .map_err(TimelineError::FFmpeg)?;

        let timeline_path = self.get_timeline_file_path(video_path);
        info!("Timeline file saved: {:?}", timeline_path);

        // 加载到缓存
        self.add_to_cache(video_path.to_path_buf(), timeline_data.clone()).await?;

        Ok(timeline_data)
    }

    /// 检查时间轴文件是否有效
    async fn is_timeline_file_valid(&self, video_path: &Path, timeline_path: &Path) -> bool {
        // 检查文件是否存在
        if !timeline_path.exists() {
            return false;
        }

        // 检查视频文件的修改时间是否比时间轴文件新
        match (
            tokio::fs::metadata(video_path).await,
            tokio::fs::metadata(timeline_path).await,
        ) {
            (Ok(video_meta), Ok(timeline_meta)) => {
                match (video_meta.modified(), timeline_meta.modified()) {
                    (Ok(video_modified), Ok(timeline_modified)) => {
                        timeline_modified >= video_modified
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// 获取时间轴文件路径
    fn get_timeline_file_path(&self, video_path: &Path) -> PathBuf {
        let mut timeline_path = video_path.to_path_buf();
        timeline_path.set_extension("timeline");
        timeline_path
    }

    /// 添加到缓存
    async fn add_to_cache(&self, video_path: PathBuf, timeline: TimelineData) -> Result<(), TimelineError> {
        // 估算时间轴数据大小
        let estimated_size = self.estimate_timeline_size(&timeline);
        
        // 检查缓存限制
        let cache_limit_bytes = (self.cache_limit_mb * 1024 * 1024) as u64;
        let mut current_size = self.current_cache_size.write().await;
        
        if *current_size + estimated_size > cache_limit_bytes {
            // 清理缓存
            self.cleanup_cache().await?;
            *current_size = self.calculate_current_cache_size().await;
        }

        // 添加到缓存
        {
            let mut cache = self.cached_timelines.write().await;
            cache.insert(video_path.clone(), timeline);
        }
        
        *current_size += estimated_size;
        
        debug!("Timeline added to cache: {:?} ({} bytes)", video_path, estimated_size);
        Ok(())
    }

    /// 估算时间轴数据大小
    fn estimate_timeline_size(&self, timeline: &TimelineData) -> u64 {
        // 粗略估算：基础结构 + 关键帧数量 * 每个关键帧的大小
        let base_size = 1024; // 基础结构大小
        let keyframe_size = 64; // 每个关键帧信息的估算大小
        base_size + (timeline.keyframes.len() as u64 * keyframe_size)
    }

    /// 计算当前缓存大小
    async fn calculate_current_cache_size(&self) -> u64 {
        let cache = self.cached_timelines.read().await;
        cache.values()
            .map(|timeline| self.estimate_timeline_size(timeline))
            .sum()
    }

    /// 清理缓存 (LRU策略)
    async fn cleanup_cache(&self) -> Result<(), TimelineError> {
        let mut cache = self.cached_timelines.write().await;
        
        // 简单策略：清理一半的缓存
        let target_size = cache.len() / 2;
        let keys_to_remove: Vec<PathBuf> = cache.keys().take(target_size).cloned().collect();
        
        for key in keys_to_remove {
            cache.remove(&key);
            debug!("Removed from cache: {:?}", key);
        }
        
        info!("Cache cleanup completed, {} entries remaining", cache.len());
        Ok(())
    }

    /// 获取缓存统计信息
    pub async fn get_cache_stats(&self) -> CacheStats {
        let cache = self.cached_timelines.read().await;
        let current_size = *self.current_cache_size.read().await;
        
        CacheStats {
            cached_files: cache.len(),
            cache_size_bytes: current_size,
            cache_limit_bytes: (self.cache_limit_mb * 1024 * 1024) as u64,
            cache_usage_percent: (current_size as f64 / (self.cache_limit_mb * 1024 * 1024) as f64 * 100.0),
        }
    }

    /// 预加载时间轴 (后台任务)
    pub async fn preload_timeline(&self, video_path: &Path) -> Result<(), TimelineError> {
        let video_path = video_path.to_path_buf();
        let manager = self.clone();
        
        tokio::spawn(async move {
            match manager.get_timeline(&video_path).await {
                Ok(_) => {
                    debug!("Timeline preloaded successfully: {:?}", video_path);
                }
                Err(e) => {
                    warn!("Failed to preload timeline for {:?}: {}", video_path, e);
                }
            }
        });
        
        Ok(())
    }

    /// 清除特定文件的缓存
    pub async fn invalidate_cache(&self, video_path: &Path) {
        let mut cache = self.cached_timelines.write().await;
        if cache.remove(video_path).is_some() {
            info!("Cache invalidated for: {:?}", video_path);
        }
    }

    /// 清除所有缓存
    pub async fn clear_cache(&self) {
        let mut cache = self.cached_timelines.write().await;
        let mut current_size = self.current_cache_size.write().await;
        
        cache.clear();
        *current_size = 0;
        
        info!("All timeline cache cleared");
    }
}

impl Clone for TimelineManager {
    fn clone(&self) -> Self {
        Self {
            ffmpeg_parser: FFmpegCliParser::new(),
            cached_timelines: self.cached_timelines.clone(),
            cache_limit_mb: self.cache_limit_mb,
            current_cache_size: self.current_cache_size.clone(),
        }
    }
}

/// 缓存统计信息
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub cached_files: usize,
    pub cache_size_bytes: u64,
    pub cache_limit_bytes: u64,
    pub cache_usage_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_timeline_manager_creation() {
        let manager = TimelineManager::new();
        let stats = manager.get_cache_stats().await;
        
        assert_eq!(stats.cached_files, 0);
        assert_eq!(stats.cache_size_bytes, 0);
    }

    #[tokio::test]
    async fn test_cache_limit_setting() {
        let mut manager = TimelineManager::new();
        manager.set_cache_limit(200);
        
        let stats = manager.get_cache_stats().await;
        assert_eq!(stats.cache_limit_bytes, 200 * 1024 * 1024);
    }

    #[test]
    fn test_timeline_file_path() {
        let manager = TimelineManager::new();
        let video_path = PathBuf::from("/path/to/video.mp4");
        let timeline_path = manager.get_timeline_file_path(&video_path);
        
        assert_eq!(timeline_path, PathBuf::from("/path/to/video.timeline"));
    }
}