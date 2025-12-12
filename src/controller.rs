use async_trait::async_trait;
use crate::types::{VideoSegment, AudioSegment, DropFrameStrategy, SyncInfo, BufferManager, KeyframeIndex, KeyframeEntry, SeekResult};
use crate::errors::PlaybackError;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[async_trait]
pub trait PlaybackController {
    async fn seek(&mut self, position: f64) -> Result<(), PlaybackError>;
    async fn seek_to_keyframe(&mut self, position: f64, index: &KeyframeIndex) -> Result<SeekResult, PlaybackError>;
    async fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlaybackError>;
    fn get_drop_frame_strategy(&self, rate: f64) -> DropFrameStrategy;
    fn adjust_transmission_queue(
        &self,
        segments: Vec<VideoSegment>,
        playback_rate: f64,
    ) -> Vec<VideoSegment>;
    fn clear_buffers(&mut self) -> Result<(), PlaybackError>;
    fn find_segment_at_position(&self, segments: &[VideoSegment], position: f64) -> Option<usize>;
    fn find_nearest_keyframe(&self, timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry>;
    fn adjust_audio_video_sync(&mut self, playback_rate: f64) -> SyncInfo;
}

pub struct DefaultPlaybackController {
    current_position: f64,
    playback_rate: f64,
    transmission_queue: VecDeque<VideoSegment>,
    audio_queue: VecDeque<AudioSegment>,
    buffer_manager: BufferManager,
    sync_offset: f64,
    last_seek_position: Option<f64>,
}

impl DefaultPlaybackController {
    pub fn new() -> Self {
        use std::collections::HashMap;
        
        Self {
            current_position: 0.0,
            playback_rate: 1.0,
            transmission_queue: VecDeque::new(),
            audio_queue: VecDeque::new(),
            buffer_manager: BufferManager {
                video_buffers: HashMap::new(),
                audio_buffers: HashMap::new(),
                max_buffer_size: 10 * 1024 * 1024, // 10MB
                current_buffer_size: 0,
                buffer_health: crate::types::BufferHealth {
                    video_buffer_level: 0.0,
                    audio_buffer_level: 0.0,
                    underrun_count: 0,
                    overrun_count: 0,
                    last_underrun: None,
                },
            },
            sync_offset: 0.0,
            last_seek_position: None,
        }
    }
}

#[async_trait]
impl PlaybackController for DefaultPlaybackController {
    async fn seek(&mut self, position: f64) -> Result<(), PlaybackError> {
        if position < 0.0 {
            return Err(PlaybackError::InvalidSeekPosition { position });
        }
        
        // Clear current transmission buffers
        self.clear_buffers()?;
        
        // Update current position
        self.current_position = position;
        self.last_seek_position = Some(position);
        
        // Reset sync offset after seek
        self.sync_offset = 0.0;
        
        Ok(())
    }

    async fn seek_to_keyframe(&mut self, position: f64, index: &KeyframeIndex) -> Result<SeekResult, PlaybackError> {
        let start_time = Instant::now();
        
        if position < 0.0 || position > index.total_duration {
            return Err(PlaybackError::InvalidSeekPosition { position });
        }
        
        // Find the nearest keyframe using the keyframe index
        let keyframe_entry = self.find_nearest_keyframe(position, index)
            .ok_or(PlaybackError::KeyframeNotFound { timestamp: position })?;
        
        // Clear current transmission buffers before seeking
        self.clear_buffers()?;
        
        // Update current position to the keyframe position
        let actual_time = keyframe_entry.timestamp;
        self.current_position = actual_time;
        self.last_seek_position = Some(actual_time);
        
        // Reset sync offset after seek
        self.sync_offset = 0.0;
        
        // Calculate seek precision achieved
        let precision_achieved = if (position - actual_time).abs() < f64::EPSILON {
            1.0 // Perfect precision
        } else {
            1.0 - ((position - actual_time).abs() / position).min(1.0)
        };
        
        let execution_time = start_time.elapsed();
        
        Ok(SeekResult {
            requested_time: position,
            actual_time,
            keyframe_offset: keyframe_entry.file_offset,
            precision_achieved,
            keyframe_used: keyframe_entry,
            execution_time,
        })
    }

    async fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlaybackError> {
        if rate <= 0.0 || rate > 10.0 {
            return Err(PlaybackError::InvalidPlaybackRate { rate });
        }
        
        let old_rate = self.playback_rate;
        self.playback_rate = rate;
        
        // Adjust audio-video synchronization when playback rate changes
        if (old_rate - rate).abs() > f64::EPSILON {
            self.adjust_audio_video_sync(rate);
        }
        
        Ok(())
    }

    fn get_drop_frame_strategy(&self, rate: f64) -> DropFrameStrategy {
        match rate {
            r if r <= 1.0 => DropFrameStrategy {
                drop_b_frames: false,
                drop_p_frames: false,
                keep_key_frames_only: false,
                adaptive_dropping: false,
            },
            r if r <= 2.0 => DropFrameStrategy {
                drop_b_frames: true,
                drop_p_frames: false,
                keep_key_frames_only: false,
                adaptive_dropping: true,
            },
            r if r <= 4.0 => DropFrameStrategy {
                drop_b_frames: true,
                drop_p_frames: true,
                keep_key_frames_only: false,
                adaptive_dropping: true,
            },
            _ => DropFrameStrategy {
                drop_b_frames: true,
                drop_p_frames: true,
                keep_key_frames_only: true,
                adaptive_dropping: true,
            },
        }
    }

    fn adjust_transmission_queue(
        &self,
        segments: Vec<VideoSegment>,
        playback_rate: f64,
    ) -> Vec<VideoSegment> {
        let mut segments = segments;
        let strategy = self.get_drop_frame_strategy(playback_rate);
        
        // Apply frame dropping strategy based on playback rate
        if strategy.keep_key_frames_only {
            segments.retain(|segment| segment.is_key_frame);
        } else {
            // Apply more sophisticated frame dropping
            if strategy.drop_b_frames || strategy.drop_p_frames {
                segments = self.apply_intelligent_frame_dropping(segments, &strategy);
            }
        }
        
        // Sort segments by timestamp to ensure proper playback order
        segments.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap_or(std::cmp::Ordering::Equal));
        
        segments
    }

    fn clear_buffers(&mut self) -> Result<(), PlaybackError> {
        // Clear transmission queues
        self.transmission_queue.clear();
        self.audio_queue.clear();
        
        // Clear buffer manager
        self.buffer_manager.video_buffers.clear();
        self.buffer_manager.audio_buffers.clear();
        self.buffer_manager.current_buffer_size = 0;
        
        // Reset buffer health metrics
        self.buffer_manager.buffer_health.video_buffer_level = 0.0;
        self.buffer_manager.buffer_health.audio_buffer_level = 0.0;
        
        Ok(())
    }

    fn find_segment_at_position(&self, segments: &[VideoSegment], position: f64) -> Option<usize> {
        // Find the segment that contains the target position
        for (index, segment) in segments.iter().enumerate() {
            let segment_start = segment.timestamp;
            let segment_end = segment.timestamp + segment.duration;
            
            if position >= segment_start && position < segment_end {
                return Some(index);
            }
        }
        
        // If exact position not found, find the closest segment before the position
        let mut closest_index = None;
        let mut closest_distance = f64::INFINITY;
        
        for (index, segment) in segments.iter().enumerate() {
            if segment.timestamp <= position {
                let distance = position - segment.timestamp;
                if distance < closest_distance {
                    closest_distance = distance;
                    closest_index = Some(index);
                }
            }
        }
        
        closest_index
    }

    fn find_nearest_keyframe(&self, timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry> {
        if index.entries.is_empty() {
            return None;
        }
        
        // Handle edge cases
        if timestamp <= 0.0 {
            return index.entries.first().cloned();
        }
        
        if timestamp >= index.total_duration {
            return index.entries.last().cloned();
        }
        
        // Binary search for the nearest keyframe at or before the timestamp
        let mut left = 0;
        let mut right = index.entries.len();
        
        while left < right {
            let mid = left + (right - left) / 2;
            
            if index.entries[mid].timestamp <= timestamp {
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        
        // left is now the index of the first keyframe after timestamp
        // We want the keyframe at or before timestamp
        if left == 0 {
            // All keyframes are after timestamp, return the first one
            index.entries.first().cloned()
        } else {
            // Return the keyframe just before or at the timestamp
            index.entries.get(left - 1).cloned()
        }
    }

    fn adjust_audio_video_sync(&mut self, playback_rate: f64) -> SyncInfo {
        // Calculate sync adjustment based on playback rate
        let base_sync_offset = self.sync_offset;
        
        // Higher playback rates may require different sync strategies
        let adjusted_offset = match playback_rate {
            r if r <= 1.0 => base_sync_offset,
            r if r <= 2.0 => base_sync_offset * 0.8, // Slight adjustment for moderate speed
            r if r <= 4.0 => base_sync_offset * 0.6, // More aggressive adjustment
            _ => 0.0, // Reset sync for very high speeds (key frames only)
        };
        
        self.sync_offset = adjusted_offset;
        
        SyncInfo {
            video_timestamp: self.current_position,
            audio_timestamp: self.current_position + adjusted_offset,
            offset: adjusted_offset,
        }
    }
}

impl DefaultPlaybackController {
    fn apply_intelligent_frame_dropping(
        &self,
        segments: Vec<VideoSegment>,
        strategy: &DropFrameStrategy,
    ) -> Vec<VideoSegment> {
        if !strategy.adaptive_dropping {
            return segments;
        }
        
        // Simulate intelligent frame dropping based on frame types
        // In a real implementation, this would analyze actual frame data
        let mut result = Vec::new();
        let mut last_key_frame_index = None;
        
        for (index, segment) in segments.iter().enumerate() {
            if segment.is_key_frame {
                // Always keep key frames
                result.push(segment.clone());
                last_key_frame_index = Some(result.len() - 1);
            } else {
                // Apply dropping strategy for non-key frames
                let should_keep = match (strategy.drop_b_frames, strategy.drop_p_frames) {
                    (false, false) => true, // Keep all frames
                    (true, false) => {
                        // Drop B-frames (simulated by dropping every 3rd non-key frame)
                        (index - last_key_frame_index.unwrap_or(0)) % 3 != 2
                    },
                    (true, true) => {
                        // Drop both B and P frames (keep only every 4th non-key frame)
                        (index - last_key_frame_index.unwrap_or(0)) % 4 == 1
                    },
                    (false, true) => {
                        // Drop P-frames (simulated by dropping every 2nd non-key frame)
                        (index - last_key_frame_index.unwrap_or(0)) % 2 == 1
                    },
                };
                
                if should_keep {
                    result.push(segment.clone());
                }
            }
        }
        
        result
    }
    
    pub fn get_current_position(&self) -> f64 {
        self.current_position
    }
    
    pub fn get_playback_rate(&self) -> f64 {
        self.playback_rate
    }
    
    pub fn get_buffer_health(&self) -> &crate::types::BufferHealth {
        &self.buffer_manager.buffer_health
    }
    
    pub fn requeue_segments_from_position(&mut self, segments: Vec<VideoSegment>, position: f64) -> Result<(), PlaybackError> {
        // Clear current queue
        self.transmission_queue.clear();
        
        // Find segments starting from the target position
        let relevant_segments: Vec<VideoSegment> = segments
            .into_iter()
            .filter(|segment| segment.timestamp >= position)
            .collect();
        
        // Apply current playback rate adjustments
        let adjusted_segments = self.adjust_transmission_queue(relevant_segments, self.playback_rate);
        
        // Add to transmission queue
        for segment in adjusted_segments {
            self.transmission_queue.push_back(segment);
        }
        
        Ok(())
    }
    
    /// Advanced playback rate control with network capacity consideration
    pub fn set_adaptive_playback_rate(&mut self, target_rate: f64, network_capacity: f64) -> Result<f64, PlaybackError> {
        if target_rate <= 0.0 || target_rate > 10.0 {
            return Err(PlaybackError::InvalidPlaybackRate { rate: target_rate });
        }
        
        // Calculate effective playback rate based on network capacity
        let effective_rate = if network_capacity > 0.0 {
            // If network can't handle the target rate, reduce it
            let max_sustainable_rate = network_capacity / 1_000_000.0; // Convert to Mbps equivalent
            target_rate.min(max_sustainable_rate)
        } else {
            target_rate
        };
        
        self.playback_rate = effective_rate;
        
        // Adjust sync strategy based on the effective rate
        self.adjust_audio_video_sync(effective_rate);
        
        Ok(effective_rate)
    }
    
    /// Get recommended frame dropping strategy based on current conditions
    pub fn get_adaptive_drop_strategy(&self, network_conditions: Option<&crate::types::NetworkConditions>) -> DropFrameStrategy {
        let base_strategy = self.get_drop_frame_strategy(self.playback_rate);
        
        // Adjust strategy based on network conditions
        if let Some(conditions) = network_conditions {
            match conditions.congestion_level {
                crate::types::CongestionLevel::Low => base_strategy,
                crate::types::CongestionLevel::Medium => DropFrameStrategy {
                    drop_b_frames: true,
                    drop_p_frames: base_strategy.drop_p_frames,
                    keep_key_frames_only: base_strategy.keep_key_frames_only,
                    adaptive_dropping: true,
                },
                crate::types::CongestionLevel::High => DropFrameStrategy {
                    drop_b_frames: true,
                    drop_p_frames: true,
                    keep_key_frames_only: base_strategy.keep_key_frames_only,
                    adaptive_dropping: true,
                },
                crate::types::CongestionLevel::Critical => DropFrameStrategy {
                    drop_b_frames: true,
                    drop_p_frames: true,
                    keep_key_frames_only: true,
                    adaptive_dropping: true,
                },
            }
        } else {
            base_strategy
        }
    }
    
    /// Calculate transmission timing based on playback rate
    pub fn calculate_transmission_timing(&self, segment: &VideoSegment) -> std::time::Duration {
        let base_duration = std::time::Duration::from_secs_f64(segment.duration);
        
        // Adjust timing based on playback rate
        let adjusted_duration = base_duration.as_secs_f64() / self.playback_rate;
        
        std::time::Duration::from_secs_f64(adjusted_duration.max(0.001)) // Minimum 1ms
    }
    
    /// Synchronize audio and video streams for different playback rates
    pub fn synchronize_av_streams(&mut self, video_segments: &[VideoSegment], audio_segments: &[AudioSegment]) -> Vec<SyncInfo> {
        let mut sync_infos = Vec::new();
        
        for video_segment in video_segments {
            // Find corresponding audio segment
            let audio_match = audio_segments.iter().find(|audio| {
                let audio_start = audio.timestamp;
                let audio_end = audio.timestamp + audio.duration;
                let video_start = video_segment.timestamp;
                
                // Check if audio and video segments overlap
                video_start >= audio_start && video_start < audio_end
            });
            
            if let Some(audio_segment) = audio_match {
                let sync_info = SyncInfo {
                    video_timestamp: video_segment.timestamp,
                    audio_timestamp: audio_segment.timestamp,
                    offset: self.calculate_sync_offset(video_segment.timestamp, audio_segment.timestamp),
                };
                sync_infos.push(sync_info);
            }
        }
        
        sync_infos
    }
    
    fn calculate_sync_offset(&self, video_timestamp: f64, audio_timestamp: f64) -> f64 {
        let base_offset = audio_timestamp - video_timestamp;
        
        // Adjust offset based on playback rate
        match self.playback_rate {
            r if r <= 1.0 => base_offset,
            r if r <= 2.0 => base_offset * 0.9, // Slight compensation for moderate speeds
            r if r <= 4.0 => base_offset * 0.7, // More compensation for higher speeds
            _ => 0.0, // No sync for very high speeds (audio may be dropped)
        }
    }
    
    /// Seek to a specific time with automatic keyframe alignment for non-keyframe positions
    pub async fn seek_with_alignment(&mut self, position: f64, index: &KeyframeIndex) -> Result<SeekResult, PlaybackError> {
        let start_time = Instant::now();
        
        if position < 0.0 || position > index.total_duration {
            return Err(PlaybackError::InvalidSeekPosition { position });
        }
        
        // Find the nearest keyframe
        let keyframe_entry = self.find_nearest_keyframe(position, index)
            .ok_or(PlaybackError::KeyframeNotFound { timestamp: position })?;
        
        // Check if the requested position is exactly at a keyframe
        let is_exact_keyframe = (keyframe_entry.timestamp - position).abs() < index.index_precision;
        
        let actual_time = if is_exact_keyframe {
            // Use the exact keyframe position
            keyframe_entry.timestamp
        } else {
            // For non-keyframe positions, align to the previous keyframe
            // This ensures decode integrity while providing the closest possible position
            keyframe_entry.timestamp
        };
        
        // Clear buffers and update position
        self.clear_buffers()?;
        self.current_position = actual_time;
        self.last_seek_position = Some(actual_time);
        self.sync_offset = 0.0;
        
        // Calculate precision achieved
        let precision_achieved = if is_exact_keyframe {
            1.0 // Perfect precision for keyframe positions
        } else {
            // Calculate how close we got to the requested position
            let distance = (position - actual_time).abs();
            let max_distance = index.index_precision;
            1.0 - (distance / max_distance).min(1.0)
        };
        
        let execution_time = start_time.elapsed();
        
        Ok(SeekResult {
            requested_time: position,
            actual_time,
            keyframe_offset: keyframe_entry.file_offset,
            precision_achieved,
            keyframe_used: keyframe_entry,
            execution_time,
        })
    }
    
    /// Find the optimal keyframe for seeking based on the optimization strategy
    pub fn find_optimal_keyframe(&self, timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry> {
        match index.optimization_strategy {
            crate::types::IndexOptimizationStrategy::Full => {
                // Use the standard nearest keyframe search for full index
                self.find_nearest_keyframe(timestamp, index)
            },
            crate::types::IndexOptimizationStrategy::Sparse => {
                // For sparse index, find the nearest available keyframe
                self.find_nearest_sparse_keyframe(timestamp, index)
            },
            crate::types::IndexOptimizationStrategy::Adaptive => {
                // Use adaptive strategy based on current memory usage
                if index.memory_usage > index.entries.len() * 64 { // Threshold for switching to sparse
                    self.find_nearest_sparse_keyframe(timestamp, index)
                } else {
                    self.find_nearest_keyframe(timestamp, index)
                }
            },
            crate::types::IndexOptimizationStrategy::Hierarchical => {
                // Use hierarchical search for better performance on large indices
                self.find_hierarchical_keyframe(timestamp, index)
            },
        }
    }
    
    /// Find nearest keyframe in a sparse index
    fn find_nearest_sparse_keyframe(&self, timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry> {
        if index.entries.is_empty() {
            return None;
        }
        
        // For sparse indices, we may need to search more broadly
        let mut best_entry = None;
        let mut best_distance = f64::INFINITY;
        
        for entry in &index.entries {
            let distance = (entry.timestamp - timestamp).abs();
            if entry.timestamp <= timestamp && distance < best_distance {
                best_distance = distance;
                best_entry = Some(entry.clone());
            }
        }
        
        // If no keyframe found before timestamp, use the first one
        best_entry.or_else(|| index.entries.first().cloned())
    }
    
    /// Find keyframe using hierarchical search for large indices
    fn find_hierarchical_keyframe(&self, timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry> {
        if index.entries.is_empty() {
            return None;
        }
        
        // For hierarchical indices, we can use a more sophisticated search
        // This is a simplified version - in practice, this would use the hierarchical structure
        let chunk_size = (index.entries.len() / 10).max(1);
        let mut candidate_chunks = Vec::new();
        
        // First, find candidate chunks
        for (i, chunk) in index.entries.chunks(chunk_size).enumerate() {
            if let (Some(first), Some(last)) = (chunk.first(), chunk.last()) {
                if timestamp >= first.timestamp && timestamp <= last.timestamp {
                    candidate_chunks.push(i);
                }
            }
        }
        
        // If no chunk contains the timestamp, find the nearest chunk before it
        if candidate_chunks.is_empty() {
            for (i, chunk) in index.entries.chunks(chunk_size).enumerate() {
                if let Some(last) = chunk.last() {
                    if last.timestamp <= timestamp {
                        candidate_chunks.push(i);
                    }
                }
            }
        }
        
        // Search within the best candidate chunk
        if let Some(&chunk_index) = candidate_chunks.last() {
            let start_idx = chunk_index * chunk_size;
            let end_idx = ((chunk_index + 1) * chunk_size).min(index.entries.len());
            
            for i in (start_idx..end_idx).rev() {
                if index.entries[i].timestamp <= timestamp {
                    return Some(index.entries[i].clone());
                }
            }
        }
        
        // Fallback to first entry
        index.entries.first().cloned()
    }
    
    /// Get detailed seek operation information
    pub fn get_seek_info(&self) -> Option<(f64, f64)> {
        self.last_seek_position.map(|pos| (pos, self.current_position))
    }
    
    /// Validate keyframe index for seek operations
    pub fn validate_keyframe_index(&self, index: &KeyframeIndex) -> Result<(), PlaybackError> {
        if index.entries.is_empty() {
            return Err(PlaybackError::InvalidKeyframeIndex { 
                reason: "Keyframe index is empty".to_string() 
            });
        }
        
        // Check if entries are sorted by timestamp
        for window in index.entries.windows(2) {
            if window[0].timestamp > window[1].timestamp {
                return Err(PlaybackError::InvalidKeyframeIndex { 
                    reason: "Keyframe entries are not sorted by timestamp".to_string() 
                });
            }
        }
        
        // Check if total duration is consistent
        if let Some(last_entry) = index.entries.last() {
            if last_entry.timestamp > index.total_duration {
                return Err(PlaybackError::InvalidKeyframeIndex { 
                    reason: "Last keyframe timestamp exceeds total duration".to_string() 
                });
            }
        }
        
        // Check precision value
        if index.index_precision <= 0.0 {
            return Err(PlaybackError::InvalidKeyframeIndex { 
                reason: "Index precision must be positive".to_string() 
            });
        }
        
        Ok(())
    }
}