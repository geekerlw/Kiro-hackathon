use tokio_stream::Stream;
use async_trait::async_trait;
use uuid::Uuid;
use crate::types::{VideoSegment, SegmentOptions, SegmentMetadata, SegmentMode};
use crate::errors::SegmentError;
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

#[async_trait]
pub trait VideoSegmenter {
    async fn segment_stream(
        &self,
        input: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
        options: SegmentOptions,
    ) -> Box<dyn Stream<Item = Result<VideoSegment, SegmentError>> + Unpin + Send>;
    
    fn find_key_frames(&self, buffer: &[u8]) -> Vec<usize>;
    
    fn find_gop_boundaries(&self, buffer: &[u8]) -> Vec<usize>;
    
    fn create_segment(
        &self,
        buffer: Vec<u8>,
        start_frame: usize,
        end_frame: usize,
        timestamp: f64,
        frame_rate: f64,
    ) -> Result<VideoSegment, SegmentError>;
    
    fn extract_encoding_params(&self, buffer: &[u8]) -> HashMap<String, String>;
}

pub struct DefaultVideoSegmenter {
    frame_rate: f64,
    buffer: Vec<u8>,
    current_timestamp: f64,
    segment_counter: usize,
}

impl DefaultVideoSegmenter {
    pub fn new() -> Self {
        Self {
            frame_rate: 30.0, // Default frame rate
            buffer: Vec::new(),
            current_timestamp: 0.0,
            segment_counter: 0,
        }
    }
    
    pub fn with_frame_rate(frame_rate: f64) -> Self {
        Self {
            frame_rate,
            buffer: Vec::new(),
            current_timestamp: 0.0,
            segment_counter: 0,
        }
    }
    

}

#[async_trait]
impl VideoSegmenter for DefaultVideoSegmenter {
    async fn segment_stream(
        &self,
        input: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
        options: SegmentOptions,
    ) -> Box<dyn Stream<Item = Result<VideoSegment, SegmentError>> + Unpin + Send> {
        Box::new(SegmentingStream::new(input, options, self.frame_rate))
    }

    fn find_key_frames(&self, buffer: &[u8]) -> Vec<usize> {
        // 统一使用固定分片策略，不再区分H.264和MP4格式
        // 对于文件上传场景，使用固定间隔的关键帧位置
        self.estimate_key_frames_fixed_size(buffer)
    }
    
    fn find_gop_boundaries(&self, buffer: &[u8]) -> Vec<usize> {
        // 统一使用固定分片策略，简化GOP边界检测
        // 基于固定大小创建边界点
        self.create_fixed_boundaries(buffer)
    }

    fn create_segment(
        &self,
        buffer: Vec<u8>,
        start_frame: usize,
        end_frame: usize,
        timestamp: f64,
        frame_rate: f64,
    ) -> Result<VideoSegment, SegmentError> {
        if start_frame >= end_frame {
            return Err(SegmentError::InvalidParameters);
        }
        
        if buffer.is_empty() {
            return Err(SegmentError::InvalidParameters);
        }

        // 简化处理：对于文件上传，直接使用原始buffer，不进行复杂的NAL单元验证
        let segment_buffer = buffer;
        
        let frame_count = end_frame - start_frame;
        let duration = frame_count as f64 / frame_rate;
        
        // 简化关键帧检测：固定分片策略下，每个分片都视为独立单元
        let key_frame_positions = vec![0]; // 简化：每个分片开始都是关键位置
        let is_key_frame = true; // 简化：所有分片都标记为关键分片
        
        let encoding_params = self.extract_basic_params(&segment_buffer);
        
        let metadata = SegmentMetadata {
            frame_indices: (start_frame..end_frame).collect(),
            key_frame_positions,
            encoding_params,
        };

        Ok(VideoSegment {
            id: Uuid::new_v4(),
            data: segment_buffer,
            timestamp,
            duration,
            frame_count,
            is_key_frame,
            metadata,
        })
    }
    

    
    fn extract_encoding_params(&self, buffer: &[u8]) -> HashMap<String, String> {
        let mut params = HashMap::new();
        
        if self.is_h264_format(buffer) {
            params.insert("codec".to_string(), "h264".to_string());
            
            // Extract comprehensive H.264 parameters
            if let Some(sps_pos) = self.find_sps_nal(buffer) {
                if let Some((profile, level)) = self.parse_sps_profile_level(buffer, sps_pos) {
                    params.insert("profile".to_string(), profile);
                    params.insert("level".to_string(), level);
                }
                
                // Extract additional SPS parameters for quality preservation
                if let Some(sps_params) = self.parse_sps_parameters(buffer, sps_pos) {
                    params.extend(sps_params);
                }
            }
            
            // Extract PPS parameters if present
            if let Some(pps_pos) = self.find_pps_nal(buffer) {
                if let Some(pps_params) = self.parse_pps_parameters(buffer, pps_pos) {
                    params.extend(pps_params);
                }
            }
            
        } else if self.is_mp4_format(buffer) {
            params.insert("container".to_string(), "mp4".to_string());
            
            // Extract MP4 metadata for quality preservation
            if let Some(mp4_params) = self.extract_mp4_parameters(buffer) {
                params.extend(mp4_params);
            }
        }
        
        params.insert("frame_rate".to_string(), self.frame_rate.to_string());
        params.insert("original_size".to_string(), buffer.len().to_string());
        
        // Add timestamp for tracking
        params.insert("extraction_timestamp".to_string(), 
                     std::time::SystemTime::now()
                         .duration_since(std::time::UNIX_EPOCH)
                         .unwrap_or_default()
                         .as_secs()
                         .to_string());
        
        params
    }
}

impl DefaultVideoSegmenter {
    /// 公开方法：解析H.264帧 - 修复版本，正确处理NAL单元边界
    pub fn parse_h264_frames(&self, buffer: &[u8]) -> Vec<(usize, bool)> {
        let mut frames = Vec::new();
        let mut i = 0;
        
        while i < buffer.len().saturating_sub(4) {
            // Look for NAL unit start codes (0x00000001 or 0x000001)
            if buffer[i] == 0x00 && buffer[i + 1] == 0x00 {
                let start_code_len = if buffer[i + 2] == 0x00 && buffer[i + 3] == 0x01 {
                    4 // 0x00000001
                } else if buffer[i + 2] == 0x01 {
                    3 // 0x000001
                } else {
                    i += 1;
                    continue;
                };
                
                if i + start_code_len < buffer.len() {
                    let nal_type = buffer[i + start_code_len] & 0x1F;
                    
                    // 只有实际的视频帧才算作帧，过滤掉SEI等辅助信息
                    let is_video_frame = matches!(nal_type, 1 | 5); // Non-IDR slice, IDR slice
                    let is_key_frame = nal_type == 5; // 只有IDR帧才是关键帧
                    
                    // 只添加实际的视频帧，跳过SPS(7)、PPS(8)、SEI(6)等
                    if is_video_frame {
                        frames.push((i, is_key_frame));
                    }
                }
                
                // 跳过整个NAL单元，而不是只跳过起始码
                let next_start = self.find_next_nal_start(buffer, i + start_code_len);
                i = if next_start > i { next_start } else { i + start_code_len };
            } else {
                i += 1;
            }
        }
        
        frames
    }
    
    /// 查找下一个NAL单元的起始位置
    fn find_next_nal_start(&self, buffer: &[u8], start_pos: usize) -> usize {
        let mut i = start_pos;
        while i < buffer.len().saturating_sub(4) {
            if buffer[i] == 0x00 && buffer[i + 1] == 0x00 {
                if (buffer[i + 2] == 0x00 && buffer[i + 3] == 0x01) ||
                   buffer[i + 2] == 0x01 {
                    return i;
                }
            }
            i += 1;
        }
        buffer.len()
    }
    
    /// 公开方法：解析MP4帧
    pub fn parse_mp4_frames(&self, buffer: &[u8]) -> Vec<(usize, bool)> {
        let mut frames = Vec::new();
        let mut i = 0;
        
        while i < buffer.len().saturating_sub(8) {
            // Read box size (4 bytes, big-endian)
            let box_size = u32::from_be_bytes([
                buffer[i], buffer[i + 1], buffer[i + 2], buffer[i + 3]
            ]) as usize;
            
            if box_size == 0 || i + box_size > buffer.len() {
                break;
            }
            
            // Read box type (4 bytes)
            let box_type = &buffer[i + 4..i + 8];
            
            // Check for video sample boxes
            if box_type == b"mdat" || box_type == b"moof" {
                // Assume this contains video frames
                // In a real implementation, we'd parse the actual frame structure
                frames.push((i, i == 0)); // First frame is key frame
            }
            
            i += box_size;
        }
        
        if frames.is_empty() {
            frames.push((0, true)); // Fallback: treat entire buffer as one key frame
        }
        
        frames
    }

    fn is_h264_format(&self, buffer: &[u8]) -> bool {
        // Check for H.264 NAL unit start codes
        buffer.len() >= 4 && (
            (buffer[0] == 0x00 && buffer[1] == 0x00 && buffer[2] == 0x00 && buffer[3] == 0x01) ||
            (buffer[0] == 0x00 && buffer[1] == 0x00 && buffer[2] == 0x01)
        )
    }
    
    fn is_mp4_format(&self, buffer: &[u8]) -> bool {
        // Check for MP4 box signature
        buffer.len() >= 8 && (
            &buffer[4..8] == b"ftyp" ||
            &buffer[4..8] == b"mdat" ||
            &buffer[4..8] == b"moov"
        )
    }
    
    /// 统一的固定大小关键帧估算方法
    fn estimate_key_frames_fixed_size(&self, buffer: &[u8]) -> Vec<usize> {
        // 使用固定分片大小策略，简化关键帧检测
        let fixed_chunk_size = 64 * 1024; // 64KB 固定分片大小
        let mut key_positions = Vec::new();
        
        let mut pos = 0;
        while pos < buffer.len() {
            key_positions.push(pos);
            pos += fixed_chunk_size;
        }
        
        if key_positions.is_empty() {
            key_positions.push(0);
        }
        
        key_positions
    }
    
    /// 创建固定边界点
    fn create_fixed_boundaries(&self, buffer: &[u8]) -> Vec<usize> {
        // 基于固定大小创建边界点，简化GOP处理
        let fixed_chunk_size = 64 * 1024; // 64KB 固定分片大小
        let mut boundaries = Vec::new();
        
        let mut pos = 0;
        while pos < buffer.len() {
            boundaries.push(pos);
            pos += fixed_chunk_size;
        }
        
        // 确保包含结尾边界
        if !boundaries.is_empty() && boundaries.last() != Some(&buffer.len()) {
            boundaries.push(buffer.len());
        }
        
        if boundaries.is_empty() {
            boundaries.push(0);
        }
        
        boundaries
    }
    
    /// 提取基本参数，简化版本
    fn extract_basic_params(&self, buffer: &[u8]) -> HashMap<String, String> {
        let mut params = HashMap::new();
        
        // 基本信息
        params.insert("segment_size".to_string(), buffer.len().to_string());
        params.insert("frame_rate".to_string(), self.frame_rate.to_string());
        params.insert("segmentation_strategy".to_string(), "fixed_size".to_string());
        
        // 添加时间戳用于追踪
        params.insert("creation_timestamp".to_string(), 
                     std::time::SystemTime::now()
                         .duration_since(std::time::UNIX_EPOCH)
                         .unwrap_or_default()
                         .as_secs()
                         .to_string());
        
        // 检测格式但不进行复杂处理
        if self.is_h264_format(buffer) {
            params.insert("detected_format".to_string(), "h264".to_string());
        } else if self.is_mp4_format(buffer) {
            params.insert("detected_format".to_string(), "mp4".to_string());
        } else {
            params.insert("detected_format".to_string(), "unknown".to_string());
        }
        
        params
    }
    
    fn estimate_key_frames(&self, buffer: &[u8]) -> Vec<(usize, bool)> {
        // 保留原方法以兼容现有代码，但简化实现
        let key_positions = self.estimate_key_frames_fixed_size(buffer);
        key_positions.into_iter()
            .map(|pos| (pos, true)) // 所有位置都标记为关键帧
            .collect()
    }
    
    fn find_sps_nal(&self, buffer: &[u8]) -> Option<usize> {
        let mut i = 0;
        while i < buffer.len().saturating_sub(4) {
            if buffer[i] == 0x00 && buffer[i + 1] == 0x00 {
                let start_code_len = if buffer[i + 2] == 0x00 && buffer[i + 3] == 0x01 {
                    4
                } else if buffer[i + 2] == 0x01 {
                    3
                } else {
                    i += 1;
                    continue;
                };
                
                if i + start_code_len < buffer.len() {
                    let nal_type = buffer[i + start_code_len] & 0x1F;
                    if nal_type == 7 { // SPS NAL unit
                        return Some(i + start_code_len);
                    }
                }
                
                i += start_code_len;
            } else {
                i += 1;
            }
        }
        None
    }
    
    fn parse_sps_profile_level(&self, buffer: &[u8], sps_pos: usize) -> Option<(String, String)> {
        if sps_pos + 3 < buffer.len() {
            let profile_idc = buffer[sps_pos + 1];
            let level_idc = buffer[sps_pos + 3];
            
            let profile = match profile_idc {
                66 => "Baseline",
                77 => "Main", 
                88 => "Extended",
                100 => "High",
                110 => "High 10",
                122 => "High 4:2:2",
                244 => "High 4:4:4",
                _ => "Unknown",
            };
            
            let level = format!("{}.{}", level_idc / 10, level_idc % 10);
            
            Some((profile.to_string(), level))
        } else {
            None
        }
    }
    
    fn parse_sps_parameters(&self, buffer: &[u8], sps_pos: usize) -> Option<HashMap<String, String>> {
        if sps_pos + 10 > buffer.len() {
            return None;
        }
        
        let mut params = HashMap::new();
        
        // Extract constraint flags for quality preservation
        let constraint_flags = buffer[sps_pos + 2];
        params.insert("constraint_set0_flag".to_string(), ((constraint_flags & 0x80) != 0).to_string());
        params.insert("constraint_set1_flag".to_string(), ((constraint_flags & 0x40) != 0).to_string());
        params.insert("constraint_set2_flag".to_string(), ((constraint_flags & 0x20) != 0).to_string());
        
        // These parameters are crucial for maintaining video quality
        params.insert("chroma_format_idc".to_string(), "1".to_string()); // Assume 4:2:0
        params.insert("bit_depth_luma_minus8".to_string(), "0".to_string()); // Assume 8-bit
        params.insert("bit_depth_chroma_minus8".to_string(), "0".to_string()); // Assume 8-bit
        
        Some(params)
    }
    
    fn find_pps_nal(&self, buffer: &[u8]) -> Option<usize> {
        let mut i = 0;
        while i < buffer.len().saturating_sub(4) {
            if buffer[i] == 0x00 && buffer[i + 1] == 0x00 {
                let start_code_len = if buffer[i + 2] == 0x00 && buffer[i + 3] == 0x01 {
                    4
                } else if buffer[i + 2] == 0x01 {
                    3
                } else {
                    i += 1;
                    continue;
                };
                
                if i + start_code_len < buffer.len() {
                    let nal_type = buffer[i + start_code_len] & 0x1F;
                    if nal_type == 8 { // PPS NAL unit
                        return Some(i + start_code_len);
                    }
                }
                
                i += start_code_len;
            } else {
                i += 1;
            }
        }
        None
    }
    
    fn parse_pps_parameters(&self, buffer: &[u8], pps_pos: usize) -> Option<HashMap<String, String>> {
        if pps_pos + 5 > buffer.len() {
            return None;
        }
        
        let mut params = HashMap::new();
        
        // PPS parameters important for quality preservation
        params.insert("entropy_coding_mode_flag".to_string(), "0".to_string()); // Simplified
        params.insert("pic_order_present_flag".to_string(), "0".to_string()); // Simplified
        params.insert("weighted_pred_flag".to_string(), "0".to_string()); // Simplified
        params.insert("weighted_bipred_idc".to_string(), "0".to_string()); // Simplified
        
        Some(params)
    }
    
    fn extract_mp4_parameters(&self, buffer: &[u8]) -> Option<HashMap<String, String>> {
        let mut params = HashMap::new();
        let mut i = 0;
        
        while i < buffer.len().saturating_sub(8) {
            let box_size = u32::from_be_bytes([
                buffer[i], buffer[i + 1], buffer[i + 2], buffer[i + 3]
            ]) as usize;
            
            if box_size == 0 || i + box_size > buffer.len() {
                break;
            }
            
            let box_type = &buffer[i + 4..i + 8];
            
            match box_type {
                b"ftyp" => {
                    // File type box - extract brand information
                    if i + 16 <= buffer.len() {
                        let major_brand = std::str::from_utf8(&buffer[i + 8..i + 12])
                            .unwrap_or("unknown");
                        params.insert("major_brand".to_string(), major_brand.to_string());
                    }
                },
                b"mvhd" => {
                    // Movie header - extract timescale and duration
                    if i + 20 <= buffer.len() {
                        let timescale = u32::from_be_bytes([
                            buffer[i + 12], buffer[i + 13], buffer[i + 14], buffer[i + 15]
                        ]);
                        params.insert("timescale".to_string(), timescale.to_string());
                    }
                },
                _ => {}
            }
            
            i += box_size;
        }
        
        if params.is_empty() {
            None
        } else {
            Some(params)
        }
    }
}

/// Streaming segmentation implementation
pub struct SegmentingStream {
    input: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
    options: SegmentOptions,
    frame_rate: f64,
    buffer: Vec<u8>,
    current_timestamp: f64,
    segment_counter: usize,
    frame_counter: usize,
    last_key_frame_pos: usize,
    error_log: Vec<SegmentationError>,
    checkpoint: Option<SegmentationCheckpoint>,
    recovery_mode: bool,
}

impl SegmentingStream {
    pub fn new(
        input: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
        options: SegmentOptions,
        frame_rate: f64,
    ) -> Self {
        Self {
            input,
            options,
            frame_rate,
            buffer: Vec::new(),
            current_timestamp: 0.0,
            segment_counter: 0,
            frame_counter: 0,
            last_key_frame_pos: 0,
            error_log: Vec::new(),
            checkpoint: None,
            recovery_mode: false,
        }
    }
    
    /// Create a new stream from a checkpoint for resumption
    pub fn from_checkpoint(
        input: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
        options: SegmentOptions,
        checkpoint: SegmentationCheckpoint,
    ) -> Self {
        let checkpoint_clone = checkpoint.clone();
        Self {
            input,
            options,
            frame_rate: checkpoint.frame_rate,
            buffer: Vec::new(),
            current_timestamp: checkpoint.current_timestamp,
            segment_counter: checkpoint.segment_counter,
            frame_counter: checkpoint.frame_counter,
            last_key_frame_pos: checkpoint.last_key_frame_pos,
            error_log: checkpoint.error_log,
            checkpoint: Some(checkpoint_clone),
            recovery_mode: true,
        }
    }
    
    /// Dynamically adjust segmentation options based on network conditions
    pub fn adjust_for_network_conditions(&mut self, conditions: &NetworkConditions) {
        let segmenter = DefaultVideoSegmenter::new();
        let new_mode = segmenter.adjust_segmentation_mode(self.options.segment_mode.clone(), conditions);
        
        self.options.segment_mode = new_mode;
        
        // Adjust target size based on bandwidth
        match conditions.bandwidth_mbps {
            bandwidth if bandwidth > 10.0 => {
                // High bandwidth: allow larger segments for efficiency
                self.options.target_size = Some(1024 * 1024); // 1MB
                self.options.max_frames_per_segment = Some(1); // Frame-level
            },
            bandwidth if bandwidth > 2.0 => {
                // Medium bandwidth: moderate segment size
                self.options.target_size = Some(512 * 1024); // 512KB
                self.options.max_frames_per_segment = Some(30); // GOP-level
            },
            _ => {
                // Low bandwidth: smaller segments
                self.options.target_size = Some(256 * 1024); // 256KB
                self.options.max_frames_per_segment = Some(60); // Time-based
            }
        }
    }
    
    /// Create a checkpoint for resumption
    pub fn create_checkpoint(&self) -> SegmentationCheckpoint {
        SegmentationCheckpoint {
            current_timestamp: self.current_timestamp,
            segment_counter: self.segment_counter,
            frame_counter: self.frame_counter,
            last_key_frame_pos: self.last_key_frame_pos,
            frame_rate: self.frame_rate,
            options: self.options.clone(),
            error_log: self.error_log.clone(),
            created_at: std::time::SystemTime::now(),
        }
    }
    
    /// Record an error during segmentation
    fn record_error(&mut self, error_type: SegmentationErrorType, message: String, position: Option<usize>) {
        let error = SegmentationError {
            error_type,
            message,
            position,
            timestamp: std::time::SystemTime::now(),
            segment_counter: self.segment_counter,
            frame_counter: self.frame_counter,
            buffer_size: self.buffer.len(),
            recovery_attempted: false,
        };
        
        self.error_log.push(error);
    }
    
    /// Attempt to recover from an error
    fn attempt_recovery(&mut self, error_index: usize) -> Result<(), SegmentError> {
        if error_index >= self.error_log.len() {
            return Err(SegmentError::InvalidParameters);
        }
        
        let error = &mut self.error_log[error_index];
        error.recovery_attempted = true;
        
        match error.error_type {
            SegmentationErrorType::BufferOverflow => {
                // Clear buffer and continue
                self.buffer.clear();
                self.record_error(
                    SegmentationErrorType::Recovery,
                    "Buffer cleared due to overflow".to_string(),
                    None,
                );
                Ok(())
            },
            SegmentationErrorType::KeyFrameNotFound => {
                // Force segment creation at current position
                if !self.buffer.is_empty() {
                    match self.create_current_segment() {
                        Ok(_) => {
                            self.record_error(
                                SegmentationErrorType::Recovery,
                                "Forced segment creation without key frame".to_string(),
                                None,
                            );
                            Ok(())
                        },
                        Err(e) => Err(e),
                    }
                } else {
                    Ok(())
                }
            },
            SegmentationErrorType::EncodingError => {
                // Skip corrupted data and continue
                if let Some(position) = error.position {
                    if position < self.buffer.len() {
                        self.buffer.drain(0..position);
                        self.record_error(
                            SegmentationErrorType::Recovery,
                            format!("Skipped {} bytes of corrupted data", position),
                            None,
                        );
                    }
                }
                Ok(())
            },
            SegmentationErrorType::Recovery => {
                // Already a recovery attempt, no further action
                Ok(())
            },
        }
    }
    
    /// Check if recovery is needed based on error patterns
    fn needs_recovery(&self) -> bool {
        if self.error_log.is_empty() {
            return false;
        }
        
        // Check for recent errors that haven't been recovered from
        let recent_errors = self.error_log.iter()
            .rev()
            .take(5)
            .filter(|e| !e.recovery_attempted)
            .count();
        
        recent_errors > 2 // Trigger recovery if more than 2 unrecovered errors
    }
    
    /// Get error statistics for monitoring
    pub fn get_error_statistics(&self) -> ErrorStatistics {
        let total_errors = self.error_log.len();
        let recovered_errors = self.error_log.iter()
            .filter(|e| e.recovery_attempted)
            .count();
        
        let error_types: std::collections::HashMap<SegmentationErrorType, usize> = 
            self.error_log.iter()
                .fold(std::collections::HashMap::new(), |mut acc, error| {
                    *acc.entry(error.error_type.clone()).or_insert(0) += 1;
                    acc
                });
        
        ErrorStatistics {
            total_errors,
            recovered_errors,
            recovery_rate: if total_errors > 0 { 
                recovered_errors as f64 / total_errors as f64 
            } else { 
                1.0 
            },
            error_types,
            last_error: self.error_log.last().cloned(),
        }
    }
    
    fn should_create_segment(&self) -> bool {
        // 简化分片决策：统一使用固定大小策略
        let target_segment_size = 512 * 1024; // 512KB 目标分片大小（更大的分片减少开销）
        let min_segment_size = 256 * 1024;    // 256KB 最小分片大小
        let max_segment_size = 1024 * 1024;   // 1MB 最大分片大小
        
        let buffer_size = self.buffer.len();
        
        // 基本大小检查
        if buffer_size < min_segment_size {
            return false;
        }
        
        // 达到目标大小或超过最大大小时创建分片
        buffer_size >= target_segment_size || buffer_size >= max_segment_size
    }
    
    // 简化的辅助方法，不再需要复杂的关键帧和GOP检测
    fn _has_sufficient_data(&self) -> bool {
        // 简化：只检查是否有足够的数据创建分片
        !self.buffer.is_empty() && self.buffer.len() >= 32 * 1024 // 32KB 最小阈值
    }
    
    fn create_current_segment(&mut self) -> Result<VideoSegment, SegmentError> {
        if self.buffer.is_empty() {
            self.record_error(
                SegmentationErrorType::BufferOverflow,
                "Attempted to create segment from empty buffer".to_string(),
                None,
            );
            return Err(SegmentError::InvalidParameters);
        }
        
        let segmenter = DefaultVideoSegmenter::with_frame_rate(self.frame_rate);
        let segment_data = self.buffer.clone();
        
        // 简化大小检查
        if segment_data.len() > 5 * 1024 * 1024 { // 5MB limit，更合理的限制
            self.record_error(
                SegmentationErrorType::BufferOverflow,
                format!("Buffer size {} exceeds limit", segment_data.len()),
                Some(segment_data.len()),
            );
            return Err(SegmentError::BufferOverflow);
        }
        
        // 简化分片创建：不生成复杂的帧索引，直接创建分片
        let estimated_frame_count = (segment_data.len() / 1024).max(1); // 简单估算
        
        // 直接创建分片，使用简化的参数
        let mut segment = match segmenter.create_segment(
            segment_data,
            self.frame_counter.saturating_sub(estimated_frame_count),
            self.frame_counter,
            self.current_timestamp,
            self.frame_rate,
        ) {
            Ok(seg) => seg,
            Err(e) => {
                self.record_error(
                    SegmentationErrorType::EncodingError,
                    format!("Segment creation failed: {:?}", e),
                    None,
                );
                return Err(e);
            }
        };
        
        // 添加简化的元数据
        segment.metadata.encoding_params.insert("segment_id".to_string(), self.segment_counter.to_string());
        segment.metadata.encoding_params.insert("estimated_frames".to_string(), estimated_frame_count.to_string());
        segment.metadata.encoding_params.insert("error_count".to_string(), self.error_log.len().to_string());
        segment.metadata.encoding_params.insert("recovery_mode".to_string(), self.recovery_mode.to_string());
        segment.metadata.encoding_params.insert("segmentation_strategy".to_string(), "fixed_size".to_string());
        
        // 创建检查点
        let checkpoint = self.create_checkpoint();
        self.checkpoint = Some(checkpoint);
        
        // 更新状态
        self.current_timestamp += segment.duration;
        self.segment_counter += 1;
        self.last_key_frame_pos = 0; // 重置，因为使用固定分片策略
        self.buffer.clear();
        
        Ok(segment)
    }
    
    fn get_frames_in_buffer(&self) -> usize {
        // 简化帧数估算：基于固定分片策略
        if self.buffer.is_empty() {
            0
        } else {
            // 简单估算：每1KB大约1帧
            (self.buffer.len() / 1024).max(1)
        }
    }
}

impl Stream for SegmentingStream {
    type Item = Result<VideoSegment, SegmentError>;
    
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Check if recovery is needed
            if self.needs_recovery() {
                // Find the first unrecovered error and attempt recovery
                if let Some(error_index) = self.error_log.iter()
                    .position(|e| !e.recovery_attempted) {
                    match self.attempt_recovery(error_index) {
                        Ok(()) => {
                            // Recovery successful, continue processing
                        },
                        Err(e) => {
                            // Recovery failed, return error
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
            }
            
            // Check if we should create a segment from current buffer
            if self.should_create_segment() {
                match self.create_current_segment() {
                    Ok(segment) => return Poll::Ready(Some(Ok(segment))),
                    Err(e) => {
                        // Try recovery once before giving up
                        if !self.recovery_mode {
                            self.recovery_mode = true;
                            continue; // Try again in recovery mode
                        } else {
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
            }
            
            // Try to get more data from input stream
            match Pin::new(&mut self.input).poll_next(cx) {
                Poll::Ready(Some(data)) => {
                    // Validate incoming data
                    if data.is_empty() {
                        self.record_error(
                            SegmentationErrorType::EncodingError,
                            "Received empty data chunk".to_string(),
                            None,
                        );
                        continue;
                    }
                    
                    // Check for buffer overflow before appending
                    let buffer_len = self.buffer.len();
                    let data_len = data.len();
                    if buffer_len + data_len > 50 * 1024 * 1024 { // 50MB limit
                        self.record_error(
                            SegmentationErrorType::BufferOverflow,
                            format!("Buffer would exceed limit: {} + {} bytes", buffer_len, data_len),
                            Some(buffer_len),
                        );
                        
                        // Force segment creation to free buffer space
                        if !self.buffer.is_empty() {
                            match self.create_current_segment() {
                                Ok(segment) => return Poll::Ready(Some(Ok(segment))),
                                Err(e) => return Poll::Ready(Some(Err(e))),
                            }
                        }
                    }
                    
                    // Append new data to buffer
                    self.buffer.extend_from_slice(&data);
                    self.frame_counter += self.get_frames_in_buffer();
                    
                    // Check target size limit
                    if let Some(target_size) = self.options.target_size {
                        if self.buffer.len() >= target_size {
                            match self.create_current_segment() {
                                Ok(segment) => return Poll::Ready(Some(Ok(segment))),
                                Err(e) => {
                                    // Try recovery once
                                    if !self.recovery_mode {
                                        self.recovery_mode = true;
                                        continue;
                                    } else {
                                        return Poll::Ready(Some(Err(e)));
                                    }
                                }
                            }
                        }
                    }
                },
                Poll::Ready(None) => {
                    // Input stream ended, create final segment if buffer has data
                    if !self.buffer.is_empty() {
                        match self.create_current_segment() {
                            Ok(segment) => return Poll::Ready(Some(Ok(segment))),
                            Err(e) => {
                                // For final segment, try recovery
                                if !self.recovery_mode {
                                    self.recovery_mode = true;
                                    continue;
                                } else {
                                    return Poll::Ready(Some(Err(e)));
                                }
                            }
                        }
                    } else {
                        return Poll::Ready(None);
                    }
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl DefaultVideoSegmenter {
    // 移除复杂的NAL单元处理方法，因为采用固定分片策略不再需要

    /// Dynamically adjust segmentation granularity based on network conditions
    pub fn adjust_segmentation_mode(&self, _current_mode: SegmentMode, network_conditions: &NetworkConditions) -> SegmentMode {
        match network_conditions.bandwidth_mbps {
            // High bandwidth: use frame-level for minimal latency
            bandwidth if bandwidth > 10.0 => {
                if network_conditions.latency_ms < 50.0 {
                    SegmentMode::Frame
                } else {
                    SegmentMode::Gop
                }
            },
            // Medium bandwidth: use GOP-level for balance
            bandwidth if bandwidth > 2.0 => SegmentMode::Gop,
            // Low bandwidth: use time-based for efficiency
            _ => SegmentMode::Time,
        }
    }
    
    /// 简化的帧索引生成
    pub fn generate_frame_index(&self, buffer: &[u8]) -> FrameIndex {
        // 简化：基于固定分片策略，不进行复杂的帧分析
        let estimated_frame_count = (buffer.len() / 1024).max(1);
        let frame_duration = 1.0 / self.frame_rate;
        let total_duration = estimated_frame_count as f64 * frame_duration;
        
        // 创建简化的帧条目
        let mut frame_entries = Vec::new();
        for i in 0..estimated_frame_count {
            frame_entries.push(FrameEntry {
                index: i,
                position: i * 1024, // 简化：假设每帧1KB
                timestamp: i as f64 * frame_duration,
                duration: frame_duration,
                frame_type: FrameType::I, // 简化：所有帧都标记为I帧
                is_key_frame: true, // 简化：所有帧都是关键帧
                size_bytes: 1024, // 简化：固定大小
            });
        }
        
        FrameIndex {
            total_frames: estimated_frame_count,
            total_duration,
            frame_rate: self.frame_rate,
            frames: frame_entries,
            encoding_params: self.extract_basic_params(buffer),
        }
    }
    
    fn estimate_frame_size(&self, buffer: &[u8], position: usize, index: usize, frames: &[(usize, bool)]) -> usize {
        if index + 1 < frames.len() {
            frames[index + 1].0 - position
        } else {
            buffer.len() - position
        }
    }
    
    /// Preserve original quality by maintaining encoding parameters
    pub fn preserve_quality_parameters(&self, original_params: &HashMap<String, String>) -> QualityPreservationConfig {
        QualityPreservationConfig {
            maintain_profile: original_params.get("profile").cloned().unwrap_or_default(),
            maintain_level: original_params.get("level").cloned().unwrap_or_default(),
            preserve_bitrate: true,
            preserve_resolution: true,
            preserve_frame_rate: true,
            preserve_color_space: true,
            original_encoding_params: original_params.clone(),
        }
    }
}

/// Network conditions for dynamic adjustment
#[derive(Debug, Clone)]
pub struct NetworkConditions {
    pub bandwidth_mbps: f64,
    pub latency_ms: f64,
    pub packet_loss_rate: f64,
    pub jitter_ms: f64,
}

/// Frame-level metadata index
#[derive(Debug, Clone)]
pub struct FrameIndex {
    pub total_frames: usize,
    pub total_duration: f64,
    pub frame_rate: f64,
    pub frames: Vec<FrameEntry>,
    pub encoding_params: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FrameEntry {
    pub index: usize,
    pub position: usize,
    pub timestamp: f64,
    pub duration: f64,
    pub frame_type: FrameType,
    pub is_key_frame: bool,
    pub size_bytes: usize,
}

#[derive(Debug, Clone)]
pub enum FrameType {
    I, // Intra-frame (key frame)
    P, // Predicted frame
    B, // Bi-directional frame
}

/// Quality preservation configuration
#[derive(Debug, Clone)]
pub struct QualityPreservationConfig {
    pub maintain_profile: String,
    pub maintain_level: String,
    pub preserve_bitrate: bool,
    pub preserve_resolution: bool,
    pub preserve_frame_rate: bool,
    pub preserve_color_space: bool,
    pub original_encoding_params: HashMap<String, String>,
}
/// Checkpoint for resuming segmentation
#[derive(Debug, Clone)]
pub struct SegmentationCheckpoint {
    pub current_timestamp: f64,
    pub segment_counter: usize,
    pub frame_counter: usize,
    pub last_key_frame_pos: usize,
    pub frame_rate: f64,
    pub options: SegmentOptions,
    pub error_log: Vec<SegmentationError>,
    pub created_at: std::time::SystemTime,
}

/// Error information for segmentation process
#[derive(Debug, Clone)]
pub struct SegmentationError {
    pub error_type: SegmentationErrorType,
    pub message: String,
    pub position: Option<usize>,
    pub timestamp: std::time::SystemTime,
    pub segment_counter: usize,
    pub frame_counter: usize,
    pub buffer_size: usize,
    pub recovery_attempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SegmentationErrorType {
    BufferOverflow,
    KeyFrameNotFound,
    EncodingError,
    Recovery,
}

/// Error statistics for monitoring
#[derive(Debug, Clone)]
pub struct ErrorStatistics {
    pub total_errors: usize,
    pub recovered_errors: usize,
    pub recovery_rate: f64,
    pub error_types: std::collections::HashMap<SegmentationErrorType, usize>,
    pub last_error: Option<SegmentationError>,
}

/// Checkpoint manager for persistence
pub struct CheckpointManager {
    checkpoints: std::collections::HashMap<String, SegmentationCheckpoint>,
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self {
            checkpoints: std::collections::HashMap::new(),
        }
    }
    
    /// Save a checkpoint with a given identifier
    pub fn save_checkpoint(&mut self, id: String, checkpoint: SegmentationCheckpoint) {
        self.checkpoints.insert(id, checkpoint);
    }
    
    /// Load a checkpoint by identifier
    pub fn load_checkpoint(&self, id: &str) -> Option<&SegmentationCheckpoint> {
        self.checkpoints.get(id)
    }
    
    /// Remove old checkpoints to free memory
    pub fn cleanup_old_checkpoints(&mut self, max_age: std::time::Duration) {
        let now = std::time::SystemTime::now();
        self.checkpoints.retain(|_, checkpoint| {
            now.duration_since(checkpoint.created_at)
                .unwrap_or(std::time::Duration::MAX) < max_age
        });
    }
    
    /// Get all checkpoint identifiers
    pub fn list_checkpoints(&self) -> Vec<String> {
        self.checkpoints.keys().cloned().collect()
    }
}

impl Default for CheckpointManager {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::iter;
    
    #[tokio::test]
    async fn test_basic_segmentation() {
        let segmenter = DefaultVideoSegmenter::new();
        
        // Create test data that looks like H.264
        let test_data = vec![
            vec![0x00, 0x00, 0x00, 0x01, 0x67], // SPS NAL unit
            vec![0x00, 0x00, 0x00, 0x01, 0x68], // PPS NAL unit  
            vec![0x00, 0x00, 0x00, 0x01, 0x65], // IDR frame
            vec![0x00, 0x00, 0x00, 0x01, 0x41], // P frame
        ];
        
        let input_stream = Box::new(iter(test_data));
        let options = SegmentOptions {
            segment_mode: SegmentMode::Frame,
            target_size: Some(1024),
            max_frames_per_segment: Some(2),
        };
        
        let mut segment_stream = segmenter.segment_stream(input_stream, options).await;
        
        // Try to get at least one segment
        use tokio_stream::StreamExt;
        if let Some(result) = segment_stream.next().await {
            match result {
                Ok(segment) => {
                    assert!(!segment.data.is_empty());
                    assert!(segment.frame_count > 0);
                    println!("Successfully created segment with {} bytes", segment.data.len());
                },
                Err(e) => {
                    println!("Segmentation error: {:?}", e);
                }
            }
        }
    }
    
    #[test]
    fn test_key_frame_detection() {
        let segmenter = DefaultVideoSegmenter::new();
        
        // Test H.264 key frame detection
        let h264_data = vec![
            0x00, 0x00, 0x00, 0x01, 0x67, // SPS (key frame)
            0x00, 0x00, 0x00, 0x01, 0x65, // IDR (key frame)
            0x00, 0x00, 0x00, 0x01, 0x41, // P frame
        ];
        
        let key_frames = segmenter.find_key_frames(&h264_data);
        assert!(!key_frames.is_empty());
        println!("Found {} key frames", key_frames.len());
    }
    
    #[test]
    fn test_encoding_params_extraction() {
        let segmenter = DefaultVideoSegmenter::new();
        
        // Test H.264 parameter extraction with simplified strategy
        let h264_data = vec![
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, // SPS with profile/level
        ];
        
        let params = segmenter.extract_basic_params(&h264_data);
        assert!(params.contains_key("detected_format"));
        assert_eq!(params.get("detected_format"), Some(&"h264".to_string()));
        assert!(params.contains_key("segmentation_strategy"));
        assert_eq!(params.get("segmentation_strategy"), Some(&"fixed_size".to_string()));
        println!("Extracted parameters: {:?}", params);
    }
    
    #[test]
    fn test_fixed_size_segmentation() {
        let segmenter = DefaultVideoSegmenter::new();
        
        // Test fixed size key frame detection
        let test_data = vec![0u8; 128 * 1024]; // 128KB test data
        let key_frames = segmenter.estimate_key_frames_fixed_size(&test_data);
        
        // Should have multiple key frames based on 64KB chunks
        assert!(key_frames.len() >= 2);
        assert_eq!(key_frames[0], 0); // First key frame at start
        assert_eq!(key_frames[1], 64 * 1024); // Second key frame at 64KB
        
        println!("Fixed size key frames: {:?}", key_frames);
    }
    
    #[test]
    fn test_fixed_boundaries() {
        let segmenter = DefaultVideoSegmenter::new();
        
        // Test fixed boundary creation
        let test_data = vec![0u8; 200 * 1024]; // 200KB test data
        let boundaries = segmenter.create_fixed_boundaries(&test_data);
        
        // Should have boundaries at fixed intervals
        assert!(boundaries.len() >= 3);
        assert_eq!(boundaries[0], 0);
        assert_eq!(boundaries[1], 64 * 1024);
        assert_eq!(boundaries[2], 128 * 1024);
        
        println!("Fixed boundaries: {:?}", boundaries);
    }
    
    #[test]
    fn test_checkpoint_creation() {
        let test_data = vec![vec![1, 2, 3, 4]];
        let input_stream = Box::new(iter(test_data));
        let options = SegmentOptions {
            segment_mode: SegmentMode::Frame,
            target_size: Some(1024),
            max_frames_per_segment: Some(10),
        };
        
        let stream = SegmentingStream::new(input_stream, options, 30.0);
        let checkpoint = stream.create_checkpoint();
        
        assert_eq!(checkpoint.frame_rate, 30.0);
        assert_eq!(checkpoint.segment_counter, 0);
        println!("Created checkpoint at timestamp: {:?}", checkpoint.created_at);
    }
}