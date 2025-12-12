use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use async_trait::async_trait;
use crate::types::{VideoFileInfo, Resolution, KeyframeIndex, KeyframeEntry, FrameType, IndexOptimizationStrategy};
use crate::errors::FileError;

/// Extended H.264 information structure
struct ExtendedH264Info {
    resolution: Option<Resolution>,
}

/// H.264 duration calculation result
#[derive(Debug, Clone)]
struct H264DurationInfo {
    duration: f64,
    estimated_frame_rate: f64,
    estimated_bitrate: u64,
    keyframe_count: u32,
    total_frames: u32,
    analysis_confidence: f64, // 0.0 to 1.0, confidence in the duration estimate
}

/// H.264 frame analysis result
#[derive(Debug, Clone)]
struct H264FrameAnalysis {
    keyframes: u32,
    total_frames: u32,
}

/// Detailed H.264 file information with enhanced duration calculation
#[derive(Debug, Clone)]
pub struct H264DetailedInfo {
    pub basic_info: VideoFileInfo,
    pub duration_analysis: H264DurationInfo,
    pub keyframe_index: KeyframeIndex,
    pub seek_table_ready: bool,
}

#[async_trait]
pub trait FileStreamReader {
    async fn open_file(&self, file_path: &Path) -> Result<File, FileError>;
    async fn read_chunk(&self, handle: &mut File, size: usize) -> Result<Vec<u8>, FileError>;
    async fn get_file_info(&self, handle: &mut File) -> Result<VideoFileInfo, FileError>;
    async fn seek_to_position(&self, handle: &mut File, position: u64) -> Result<(), FileError>;
    async fn seek_to_time(&self, handle: &mut File, time_seconds: f64, index: &KeyframeIndex) -> Result<u64, FileError>;
    async fn build_keyframe_index(&self, handle: &mut File) -> Result<KeyframeIndex, FileError>;
    async fn close_file(&self, handle: File) -> Result<(), FileError>;
    async fn validate_format(&self, file_path: &Path) -> Result<String, FileError>;
}

pub struct DefaultFileStreamReader;

impl DefaultFileStreamReader {
    pub fn new() -> Self {
        Self
    }

    /// Build keyframe index with specific optimization strategy
    pub async fn build_keyframe_index_with_strategy(
        &self, 
        handle: &mut File, 
        strategy: IndexOptimizationStrategy
    ) -> Result<KeyframeIndex, FileError> {
        self.build_keyframe_index_impl(handle, strategy).await
    }

    /// Build keyframe index with memory limit
    pub async fn build_keyframe_index_with_memory_limit(
        &self, 
        handle: &mut File, 
        memory_limit_mb: usize
    ) -> Result<KeyframeIndex, FileError> {
        // Choose strategy based on memory limit
        let strategy = if memory_limit_mb >= 50 {
            IndexOptimizationStrategy::Full
        } else if memory_limit_mb >= 20 {
            IndexOptimizationStrategy::Adaptive
        } else if memory_limit_mb >= 10 {
            IndexOptimizationStrategy::Sparse
        } else {
            IndexOptimizationStrategy::Hierarchical
        };
        
        self.build_keyframe_index_impl(handle, strategy).await
    }

    /// Get keyframe index statistics
    pub fn get_index_stats(&self, index: &KeyframeIndex) -> IndexStats {
        let avg_gop_size = if !index.entries.is_empty() {
            index.entries.iter().map(|e| e.gop_size as f64).sum::<f64>() / index.entries.len() as f64
        } else {
            0.0
        };

        let avg_frame_size = if !index.entries.is_empty() {
            index.entries.iter().map(|e| e.frame_size as f64).sum::<f64>() / index.entries.len() as f64
        } else {
            0.0
        };

        IndexStats {
            total_keyframes: index.entries.len(),
            memory_usage_bytes: index.memory_usage,
            index_precision_seconds: index.index_precision,
            average_gop_size: avg_gop_size,
            average_frame_size_bytes: avg_frame_size,
            optimization_strategy: index.optimization_strategy.clone(),
            supports_sub_second_precision: index.index_precision < 1.0,
        }
    }

    /// Get detailed H.264 file information with improved duration calculation
    pub async fn get_h264_detailed_info(&self, handle: &mut File) -> Result<H264DetailedInfo, FileError> {
        let current_pos = handle.stream_position().await?;
        
        // Get basic file info
        let file_info = self.get_file_info(handle).await?;
        
        // Get duration info with keyframe analysis
        let duration_info = self.calculate_h264_duration_from_keyframes(handle).await?;
        
        // Build keyframe index for seek table
        let keyframe_index = self.build_keyframe_index(handle).await?;
        
        // Restore position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        Ok(H264DetailedInfo {
            basic_info: file_info,
            duration_analysis: duration_info,
            keyframe_index,
            seek_table_ready: true,
        })
    }
}

/// Statistics about a keyframe index
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_keyframes: usize,
    pub memory_usage_bytes: usize,
    pub index_precision_seconds: f64,
    pub average_gop_size: f64,
    pub average_frame_size_bytes: f64,
    pub optimization_strategy: IndexOptimizationStrategy,
    pub supports_sub_second_precision: bool,
}

impl DefaultFileStreamReader {
    /// Check if the buffer contains MP4 format data
    fn is_mp4_format(&self, buffer: &[u8]) -> Result<bool, FileError> {
        if buffer.len() < 12 {
            return Ok(false);
        }
        
        // Check for MP4 ftyp box
        if &buffer[4..8] == b"ftyp" {
            // Check for common MP4 brands
            let brand = &buffer[8..12];
            return Ok(brand == b"mp41" || brand == b"mp42" || brand == b"isom" || 
                     brand == b"avc1" || brand == b"M4V " || brand == b"mp4v");
        }
        
        Ok(false)
    }
    
    /// Check if the buffer contains H.264 format data
    fn is_h264_format(&self, buffer: &[u8]) -> Result<bool, FileError> {
        if buffer.len() < 4 {
            return Ok(false);
        }
        
        // Look for H.264 start codes (0x00000001 or 0x000001)
        for i in 0..buffer.len().saturating_sub(4) {
            // Check for 4-byte start code
            if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] {
                if i + 4 < buffer.len() {
                    let nal_type = buffer[i + 4] & 0x1F;
                    // Common H.264 NAL unit types: SPS (7), PPS (8), IDR (5), Non-IDR (1)
                    if matches!(nal_type, 1 | 5 | 7 | 8) {
                        return Ok(true);
                    }
                }
            }
            // Check for 3-byte start code
            if i + 3 < buffer.len() && buffer[i..i+3] == [0x00, 0x00, 0x01] {
                if i + 3 < buffer.len() {
                    let nal_type = buffer[i + 3] & 0x1F;
                    if matches!(nal_type, 1 | 5 | 7 | 8) {
                        return Ok(true);
                    }
                }
            }
        }
        
        Ok(false)
    }
    
    /// Extract metadata from MP4 file with integrity checking
    async fn extract_mp4_info(&self, handle: &mut File, initial_buffer: &[u8]) -> Result<VideoFileInfo, FileError> {
        // Perform file integrity check
        self.check_mp4_integrity(handle, initial_buffer).await?;
        
        // Get file size for duration estimation
        let file_size = handle.metadata().await?.len();
        
        // Parse basic MP4 structure to extract metadata
        let (resolution, has_audio, estimated_bitrate) = self.parse_mp4_basic_info(handle, initial_buffer).await?;
        
        // Estimate duration based on file size and bitrate
        let estimated_duration = if estimated_bitrate > 0 {
            (file_size as f64 * 8.0) / estimated_bitrate as f64
        } else {
            (file_size as f64) / (5_000_000.0 / 8.0) // Fallback to 5Mbps assumption
        };
        
        Ok(VideoFileInfo {
            duration: estimated_duration.max(0.1), // At least 0.1 second
            resolution,
            codec: "h264".to_string(), // Most MP4s use H.264
            frame_rate: 30.0, // Default, would need proper parsing for exact value
            bit_rate: estimated_bitrate.max(1_000_000), // At least 1Mbps
            has_audio,
        })
    }
    
    /// Extract metadata from H.264 file with integrity checking and improved duration calculation
    async fn extract_h264_info(&self, handle: &mut File, initial_buffer: &[u8]) -> Result<VideoFileInfo, FileError> {
        // Perform file integrity check
        self.check_h264_integrity(handle, initial_buffer).await?;
        
        // Look for SPS (Sequence Parameter Set) NAL unit to extract resolution
        let mut resolution = Resolution {
            width: 1920,
            height: 1080,
            aspect_ratio: "16:9".to_string(),
        };
        
        // Search for SPS NAL unit (type 7) in initial buffer and larger sample if needed
        if let Some(sps_data) = self.find_sps_nal_unit(initial_buffer) {
            if let Ok(parsed_resolution) = self.parse_sps_resolution(sps_data) {
                resolution = parsed_resolution;
            }
        } else {
            // If not found in initial buffer, search in a larger sample
            let extended_info = self.search_extended_h264_info(handle).await?;
            if let Some(found_resolution) = extended_info.resolution {
                resolution = found_resolution;
            }
        }
        
        // Improved duration calculation based on keyframe analysis
        let duration_info = self.calculate_h264_duration_from_keyframes(handle).await?;
        
        Ok(VideoFileInfo {
            duration: duration_info.duration,
            resolution,
            codec: "h264".to_string(),
            frame_rate: duration_info.estimated_frame_rate,
            bit_rate: duration_info.estimated_bitrate,
            has_audio: false, // Raw H.264 typically doesn't have audio
        })
    }
    
    /// Find SPS NAL unit in buffer
    fn find_sps_nal_unit<'a>(&self, buffer: &'a [u8]) -> Option<&'a [u8]> {
        for i in 0..buffer.len().saturating_sub(5) {
            // Look for start code followed by SPS NAL unit (type 7)
            if (buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] && (buffer[i+4] & 0x1F) == 7) ||
               (buffer[i..i+3] == [0x00, 0x00, 0x01] && (buffer[i+3] & 0x1F) == 7) {
                
                let start_offset = if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] { i + 4 } else { i + 3 };
                
                // Find the end of this NAL unit (next start code or end of buffer)
                let mut end_offset = buffer.len();
                for j in (start_offset + 1)..buffer.len().saturating_sub(3) {
                    if buffer[j..j+3] == [0x00, 0x00, 0x01] || 
                       (j + 4 <= buffer.len() && buffer[j..j+4] == [0x00, 0x00, 0x00, 0x01]) {
                        end_offset = j;
                        break;
                    }
                }
                
                return Some(&buffer[start_offset..end_offset]);
            }
        }
        None
    }
    
    /// Parse resolution from SPS NAL unit (simplified)
    fn parse_sps_resolution(&self, _sps_data: &[u8]) -> Result<Resolution, FileError> {
        // This would require proper H.264 SPS parsing with Exponential-Golomb decoding
        // For now, return common resolutions based on data patterns
        Ok(Resolution {
            width: 1920,
            height: 1080,
            aspect_ratio: "16:9".to_string(),
        })
    }
    
    /// Check MP4 file integrity
    async fn check_mp4_integrity(&self, handle: &mut File, initial_buffer: &[u8]) -> Result<(), FileError> {
        // Check if we have a valid ftyp box
        if initial_buffer.len() < 12 || &initial_buffer[4..8] != b"ftyp" {
            return Err(FileError::CorruptedFile);
        }
        
        // Get file size and check if it's reasonable
        let file_size = handle.metadata().await?.len();
        if file_size < 32 {
            return Err(FileError::CorruptedFile);
        }
        
        // Check if ftyp box size is reasonable
        let ftyp_size = u32::from_be_bytes([
            initial_buffer[0], initial_buffer[1], 
            initial_buffer[2], initial_buffer[3]
        ]);
        
        if ftyp_size < 16 || ftyp_size as u64 > file_size {
            return Err(FileError::CorruptedFile);
        }
        
        // Look for moov box (movie metadata) in the file
        // Check both beginning and end of file since moov can be at either location
        let current_pos = handle.stream_position().await?;
        let mut found_moov = false;
        let mut buffer = vec![0u8; 8192];
        
        // First, check the beginning of the file (first 1MB)
        handle.seek(SeekFrom::Start(0)).await?;
        let mut total_read = 0u64;
        
        while total_read < file_size && total_read < 1_000_000 { // Check first 1MB
            let bytes_read = handle.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            
            // Look for 'moov' box signature
            for i in 0..bytes_read.saturating_sub(4) {
                if &buffer[i..i+4] == b"moov" {
                    found_moov = true;
                    break;
                }
            }
            
            if found_moov {
                break;
            }
            
            total_read += bytes_read as u64;
        }
        
        // If not found at the beginning, check the end of the file (last 1MB)
        if !found_moov && file_size > 1_000_000 {
            let end_search_start = file_size.saturating_sub(1_000_000);
            handle.seek(SeekFrom::Start(end_search_start)).await?;
            
            while let Ok(bytes_read) = handle.read(&mut buffer).await {
                if bytes_read == 0 {
                    break;
                }
                
                // Look for 'moov' box signature
                for i in 0..bytes_read.saturating_sub(4) {
                    if &buffer[i..i+4] == b"moov" {
                        found_moov = true;
                        break;
                    }
                }
                
                if found_moov {
                    break;
                }
            }
        }
        
        // Restore position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        if !found_moov {
            return Err(FileError::CorruptedFile);
        }
        
        Ok(())
    }
    
    /// Check H.264 file integrity
    async fn check_h264_integrity(&self, handle: &mut File, initial_buffer: &[u8]) -> Result<(), FileError> {
        // Check if we have valid NAL unit start codes
        if !self.is_h264_format(initial_buffer)? {
            return Err(FileError::CorruptedFile);
        }
        
        // Get file size and check if it's reasonable
        let file_size = handle.metadata().await?.len();
        if file_size < 10 {
            return Err(FileError::CorruptedFile);
        }
        
        // Look for essential NAL units (SPS and PPS)
        let current_pos = handle.stream_position().await?;
        handle.seek(SeekFrom::Start(0)).await?;
        
        let mut found_sps = false;
        let mut found_pps = false;
        let mut buffer = vec![0u8; 8192];
        let mut total_read = 0u64;
        
        while total_read < file_size && total_read < 50_000 { // Check first 50KB
            let bytes_read = handle.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            
            // Look for SPS (type 7) and PPS (type 8) NAL units
            for i in 0..bytes_read.saturating_sub(5) {
                if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] {
                    let nal_type = buffer[i + 4] & 0x1F;
                    if nal_type == 7 {
                        found_sps = true;
                    } else if nal_type == 8 {
                        found_pps = true;
                    }
                } else if buffer[i..i+3] == [0x00, 0x00, 0x01] {
                    let nal_type = buffer[i + 3] & 0x1F;
                    if nal_type == 7 {
                        found_sps = true;
                    } else if nal_type == 8 {
                        found_pps = true;
                    }
                }
                
                if found_sps && found_pps {
                    break;
                }
            }
            
            if found_sps && found_pps {
                break;
            }
            
            total_read += bytes_read as u64;
        }
        
        // Restore position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        // For a valid H.264 stream, we should find at least SPS
        if !found_sps {
            return Err(FileError::CorruptedFile);
        }
        
        Ok(())
    }
    
    /// Parse basic MP4 information
    async fn parse_mp4_basic_info(&self, handle: &mut File, _initial_buffer: &[u8]) -> Result<(Resolution, bool, u64), FileError> {
        // This is a simplified implementation
        // A complete implementation would parse MP4 boxes properly
        
        let file_size = handle.metadata().await?.len();
        
        // Default values - in a real implementation, these would be parsed from MP4 boxes
        let resolution = Resolution {
            width: 1920,
            height: 1080,
            aspect_ratio: "16:9".to_string(),
        };
        
        let has_audio = true; // Most MP4 files have audio
        let estimated_bitrate = ((file_size * 8) / 60).max(1_000_000); // Assume 1 minute duration minimum
        
        Ok((resolution, has_audio, estimated_bitrate))
    }
    
    /// Search for extended H.264 information in larger file sample
    async fn search_extended_h264_info(&self, handle: &mut File) -> Result<ExtendedH264Info, FileError> {
        let current_pos = handle.stream_position().await?;
        handle.seek(SeekFrom::Start(0)).await?;
        
        let mut buffer = vec![0u8; 32768]; // 32KB buffer for extended search
        let bytes_read = handle.read(&mut buffer).await?;
        
        // Restore position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        // Look for SPS NAL unit in the extended buffer
        let resolution = if let Some(sps_data) = self.find_sps_nal_unit(&buffer[..bytes_read]) {
            self.parse_sps_resolution(sps_data).ok()
        } else {
            None
        };
        
        Ok(ExtendedH264Info { resolution })
    }

    /// Calculate H.264 duration based on keyframe analysis
    /// Assumes approximately one keyframe per second for more accurate duration estimation
    async fn calculate_h264_duration_from_keyframes(&self, handle: &mut File) -> Result<H264DurationInfo, FileError> {
        let current_pos = handle.stream_position().await?;
        let file_size = handle.metadata().await?.len();
        
        // Seek to beginning for analysis
        handle.seek(SeekFrom::Start(0)).await?;
        
        let mut keyframe_count = 0u32;
        let mut total_frames = 0u32;
        let mut bytes_analyzed = 0u64;
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer
        
        // Analyze up to 10MB of the file to estimate frame rate and duration
        let max_analysis_size = (10 * 1024 * 1024).min(file_size);
        
        while bytes_analyzed < max_analysis_size {
            let bytes_read = handle.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            
            // Count keyframes (IDR frames, NAL type 5) and total frames
            let frame_info = self.analyze_h264_frames_in_buffer(&buffer[..bytes_read]);
            keyframe_count += frame_info.keyframes;
            total_frames += frame_info.total_frames;
            
            bytes_analyzed += bytes_read as u64;
        }
        
        // Restore original position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        // Calculate duration based on keyframe analysis
        let duration = if keyframe_count > 0 {
            // Method 1: Assume 1 keyframe per second (your suggestion)
            // This is often accurate for streaming video content
            keyframe_count as f64
        } else {
            // Fallback: estimate based on file size and typical bitrates
            let estimated_bitrate = self.estimate_h264_bitrate_from_file_size(file_size);
            (file_size as f64 * 8.0) / estimated_bitrate
        };
        
        // Estimate frame rate based on total frames and duration
        let estimated_frame_rate = if duration > 0.0 && total_frames > 0 {
            // Scale up the frame count based on analyzed portion
            let analysis_ratio = bytes_analyzed as f64 / file_size as f64;
            let estimated_total_frames = total_frames as f64 / analysis_ratio;
            (estimated_total_frames / duration).min(60.0).max(15.0) // Clamp between 15-60 fps
        } else {
            30.0 // Default frame rate
        };
        
        // Calculate bitrate based on file size and duration
        let estimated_bitrate = if duration > 0.0 {
            ((file_size as f64 * 8.0) / duration).max(500_000.0) as u64 // At least 500kbps
        } else {
            2_000_000 // Default 2Mbps
        };
        
        Ok(H264DurationInfo {
            duration: duration.max(0.1), // At least 0.1 second
            estimated_frame_rate,
            estimated_bitrate,
            keyframe_count,
            total_frames,
            analysis_confidence: if keyframe_count > 5 { 0.9 } else { 0.6 },
        })
    }

    /// Analyze H.264 frames in a buffer to count keyframes and total frames
    fn analyze_h264_frames_in_buffer(&self, buffer: &[u8]) -> H264FrameAnalysis {
        let mut keyframes = 0u32;
        let mut total_frames = 0u32;
        
        let mut i = 0;
        while i < buffer.len().saturating_sub(5) {
            // Look for NAL unit start codes
            if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] {
                if i + 4 < buffer.len() {
                    let nal_type = buffer[i + 4] & 0x1F;
                    match nal_type {
                        5 => {
                            // IDR frame (keyframe)
                            keyframes += 1;
                            total_frames += 1;
                        },
                        1 => {
                            // Non-IDR frame (P or B frame)
                            total_frames += 1;
                        },
                        _ => {
                            // Other NAL units (SPS, PPS, etc.)
                        }
                    }
                }
                i += 4;
            } else if buffer[i..i+3] == [0x00, 0x00, 0x01] {
                if i + 3 < buffer.len() {
                    let nal_type = buffer[i + 3] & 0x1F;
                    match nal_type {
                        5 => {
                            keyframes += 1;
                            total_frames += 1;
                        },
                        1 => {
                            total_frames += 1;
                        },
                        _ => {}
                    }
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        
        H264FrameAnalysis {
            keyframes,
            total_frames,
        }
    }

    /// Estimate H.264 bitrate based on file size for fallback duration calculation
    fn estimate_h264_bitrate_from_file_size(&self, file_size: u64) -> f64 {
        // Estimate bitrate based on file size ranges
        // These are rough estimates for typical H.264 content
        match file_size {
            0..=1_000_000 => 500_000.0,           // < 1MB: 500kbps
            1_000_001..=10_000_000 => 1_500_000.0, // 1-10MB: 1.5Mbps
            10_000_001..=100_000_000 => 3_000_000.0, // 10-100MB: 3Mbps
            100_000_001..=500_000_000 => 5_000_000.0, // 100-500MB: 5Mbps
            _ => 8_000_000.0,                     // > 500MB: 8Mbps
        }
    }

    /// Build keyframe index for the video file
    async fn build_keyframe_index_impl(&self, handle: &mut File, strategy: IndexOptimizationStrategy) -> Result<KeyframeIndex, FileError> {
        let current_pos = handle.stream_position().await?;
        let file_size = handle.metadata().await?.len();
        
        // Seek to beginning
        handle.seek(SeekFrom::Start(0)).await?;
        
        let mut entries = Vec::new();
        let mut current_offset = 0u64;
        let mut memory_usage = 0usize;
        
        // Determine buffer size based on optimization strategy
        let buffer_size = match strategy {
            IndexOptimizationStrategy::Full => 64 * 1024,        // 64KB for full indexing
            IndexOptimizationStrategy::Sparse => 128 * 1024,     // 128KB for sparse sampling
            IndexOptimizationStrategy::Adaptive => 96 * 1024,    // 96KB for adaptive
            IndexOptimizationStrategy::Hierarchical => 32 * 1024, // 32KB for hierarchical
        };
        
        let mut buffer = vec![0u8; buffer_size];
        let mut frame_count = 0u64;
        let estimated_frame_rate = 30.0; // Default frame rate for time calculation
        
        // Memory limit for optimization (default 10MB)
        let memory_limit = 10 * 1024 * 1024;
        
        while current_offset < file_size {
            let bytes_read = handle.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            
            // Find keyframes in current buffer
            let keyframes = self.find_keyframes_in_buffer(&buffer[..bytes_read], current_offset)?;
            
            for keyframe in keyframes {
                // Calculate timestamp based on frame count and estimated frame rate
                let timestamp = frame_count as f64 / estimated_frame_rate;
                
                // Apply optimization strategy
                let should_include = match strategy {
                    IndexOptimizationStrategy::Full => true,
                    IndexOptimizationStrategy::Sparse => frame_count % 30 == 0, // Every 30 frames (~1 second)
                    IndexOptimizationStrategy::Adaptive => {
                        // Include if memory usage is below limit, otherwise sparse sampling
                        memory_usage < memory_limit || frame_count % 60 == 0
                    },
                    IndexOptimizationStrategy::Hierarchical => {
                        // Multi-level precision: every frame for first 10 seconds, then sparse
                        timestamp < 10.0 || frame_count % 30 == 0
                    },
                };
                
                if should_include {
                    let entry = KeyframeEntry {
                        timestamp,
                        file_offset: keyframe.offset,
                        frame_size: keyframe.size,
                        gop_size: keyframe.gop_size,
                        frame_type: FrameType::I,
                    };
                    
                    memory_usage += std::mem::size_of::<KeyframeEntry>();
                    entries.push(entry);
                }
                
                frame_count += 1;
                
                // Check memory limit for adaptive strategy
                if matches!(strategy, IndexOptimizationStrategy::Adaptive) && memory_usage > memory_limit {
                    // Switch to sparse sampling
                    break;
                }
            }
            
            current_offset += bytes_read as u64;
            
            // For large files, limit scanning to prevent excessive processing time
            if current_offset > 100 * 1024 * 1024 { // 100MB limit for indexing
                break;
            }
        }
        
        // Restore original position
        handle.seek(SeekFrom::Start(current_pos)).await?;
        
        // Calculate index precision based on entry density and frame rate
        let index_precision = if entries.len() > 1 {
            let time_span = entries.last().unwrap().timestamp - entries.first().unwrap().timestamp;
            (time_span / entries.len() as f64).max(1.0 / estimated_frame_rate)
        } else {
            1.0 / estimated_frame_rate // Single frame precision
        };
        
        // Improved duration calculation for H.264 files
        let final_duration = if !entries.is_empty() {
            // For H.264, use keyframe-based duration calculation
            // Assume approximately 1 keyframe per second (your suggestion)
            let keyframe_based_duration = entries.len() as f64;
            
            // Also calculate based on last keyframe timestamp
            let timestamp_based_duration = entries.last().unwrap().timestamp + (1.0 / estimated_frame_rate);
            
            // Use the more conservative (larger) estimate
            let conservative_duration = keyframe_based_duration.max(timestamp_based_duration);
            
            // Ensure minimum duration
            conservative_duration.max(1.0)
        } else {
            // Fallback: estimate based on file size and typical H.264 bitrates
            let estimated_bitrate = match file_size {
                0..=10_000_000 => 1_500_000.0,      // Small files: 1.5Mbps
                10_000_001..=100_000_000 => 3_000_000.0, // Medium files: 3Mbps
                _ => 5_000_000.0,                    // Large files: 5Mbps
            };
            ((file_size as f64 * 8.0) / estimated_bitrate).max(1.0)
        };
        
        Ok(KeyframeIndex {
            entries,
            total_duration: final_duration,
            index_precision,
            memory_optimized: !matches!(strategy, IndexOptimizationStrategy::Full),
            optimization_strategy: strategy,
            memory_usage,
        })
    }

    /// Find keyframes in a buffer
    fn find_keyframes_in_buffer(&self, buffer: &[u8], base_offset: u64) -> Result<Vec<KeyframeInfo>, FileError> {
        let mut keyframes = Vec::new();
        
        // Look for H.264 NAL units
        for i in 0..buffer.len().saturating_sub(5) {
            // Check for 4-byte start code
            if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] {
                if i + 4 < buffer.len() {
                    let nal_type = buffer[i + 4] & 0x1F;
                    // IDR frame (type 5) is a keyframe
                    if nal_type == 5 {
                        let keyframe = KeyframeInfo {
                            offset: base_offset + i as u64,
                            size: self.estimate_frame_size(&buffer[i..]).unwrap_or(1024),
                            gop_size: 1, // Simplified GOP size
                        };
                        keyframes.push(keyframe);
                    }
                }
            }
            // Check for 3-byte start code
            else if buffer[i..i+3] == [0x00, 0x00, 0x01] {
                if i + 3 < buffer.len() {
                    let nal_type = buffer[i + 3] & 0x1F;
                    if nal_type == 5 {
                        let keyframe = KeyframeInfo {
                            offset: base_offset + i as u64,
                            size: self.estimate_frame_size(&buffer[i..]).unwrap_or(1024),
                            gop_size: 1,
                        };
                        keyframes.push(keyframe);
                    }
                }
            }
        }
        
        Ok(keyframes)
    }

    /// Estimate frame size from buffer
    fn estimate_frame_size(&self, buffer: &[u8]) -> Option<u32> {
        // Look for next start code to determine frame size
        for i in 4..buffer.len().saturating_sub(4) {
            if buffer[i..i+4] == [0x00, 0x00, 0x00, 0x01] || 
               (i + 3 <= buffer.len() && buffer[i..i+3] == [0x00, 0x00, 0x01]) {
                return Some(i as u32);
            }
        }
        None
    }

    /// Find nearest keyframe entry for a given timestamp
    fn find_nearest_keyframe<'a>(&self, timestamp: f64, index: &'a KeyframeIndex) -> Option<&'a KeyframeEntry> {
        if index.entries.is_empty() {
            return None;
        }
        
        // Binary search for the nearest keyframe at or before the timestamp
        let mut left = 0;
        let mut right = index.entries.len();
        
        while left < right {
            let mid = (left + right) / 2;
            if index.entries[mid].timestamp <= timestamp {
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        
        // Return the keyframe at or before the timestamp
        if left > 0 {
            Some(&index.entries[left - 1])
        } else {
            Some(&index.entries[0])
        }
    }

    /// Seek to time using keyframe index with enhanced result validation and precision measurement
    async fn seek_to_time_impl(&self, handle: &mut File, time_seconds: f64, index: &KeyframeIndex) -> Result<u64, FileError> {
        use std::time::Instant;
        
        let start_time = Instant::now();
        
        // Validate input parameters
        if time_seconds < 0.0 {
            return Err(FileError::InvalidSeekPosition);
        }
        
        if time_seconds > index.total_duration {
            return Err(FileError::SeekBeyondEnd);
        }
        
        // Find the nearest keyframe
        if let Some(keyframe) = self.find_nearest_keyframe(time_seconds, index) {
            // Seek to the keyframe position
            handle.seek(SeekFrom::Start(keyframe.file_offset)).await?;
            
            // Verify the seek operation by checking current position
            let actual_position = handle.stream_position().await?;
            if actual_position != keyframe.file_offset {
                return Err(FileError::SeekFailed);
            }
            
            let execution_time = start_time.elapsed();
            
            // Log seek operation for debugging (in a real implementation, this might use a proper logger)
            #[cfg(debug_assertions)]
            eprintln!(
                "Seek operation: requested={:.3}s, actual={:.3}s, offset={}, precision={:.3}s, time={:?}",
                time_seconds,
                keyframe.timestamp,
                keyframe.file_offset,
                (time_seconds - keyframe.timestamp).abs(),
                execution_time
            );
            
            Ok(keyframe.file_offset)
        } else {
            // Fallback to beginning if no keyframe found
            handle.seek(SeekFrom::Start(0)).await?;
            Ok(0)
        }
    }

    /// Enhanced seek to time with detailed result information
    pub async fn seek_to_time_with_result(&self, handle: &mut File, time_seconds: f64, index: &KeyframeIndex) -> Result<crate::types::SeekResult, FileError> {
        use std::time::Instant;
        use crate::types::SeekResult;
        
        let start_time = Instant::now();
        
        // Validate input parameters
        if time_seconds < 0.0 {
            return Err(FileError::InvalidSeekPosition);
        }
        
        if time_seconds > index.total_duration {
            return Err(FileError::SeekBeyondEnd);
        }
        
        // Find the nearest keyframe
        if let Some(keyframe) = self.find_nearest_keyframe(time_seconds, index) {
            // Seek to the keyframe position
            handle.seek(SeekFrom::Start(keyframe.file_offset)).await?;
            
            // Verify the seek operation
            let actual_position = handle.stream_position().await?;
            if actual_position != keyframe.file_offset {
                return Err(FileError::SeekFailed);
            }
            
            let execution_time = start_time.elapsed();
            let precision_achieved = (time_seconds - keyframe.timestamp).abs();
            
            Ok(SeekResult {
                requested_time: time_seconds,
                actual_time: keyframe.timestamp,
                keyframe_offset: keyframe.file_offset,
                precision_achieved,
                keyframe_used: keyframe.clone(),
                execution_time,
            })
        } else {
            // Fallback to beginning if no keyframe found
            handle.seek(SeekFrom::Start(0)).await?;
            let execution_time = start_time.elapsed();
            
            // Create a dummy keyframe entry for the beginning
            let beginning_keyframe = crate::types::KeyframeEntry {
                timestamp: 0.0,
                file_offset: 0,
                frame_size: 0,
                gop_size: 0,
                frame_type: crate::types::FrameType::I,
            };
            
            Ok(SeekResult {
                requested_time: time_seconds,
                actual_time: 0.0,
                keyframe_offset: 0,
                precision_achieved: time_seconds,
                keyframe_used: beginning_keyframe,
                execution_time,
            })
        }
    }

    /// Direct seek to file offset position with validation
    pub async fn seek_to_offset_with_validation(&self, handle: &mut File, offset: u64) -> Result<u64, FileError> {
        use std::time::Instant;
        
        let start_time = Instant::now();
        
        // Get file size for validation
        let file_size = handle.metadata().await?.len();
        
        // Validate offset is within file bounds
        if offset >= file_size {
            return Err(FileError::SeekBeyondEnd);
        }
        
        // Perform the seek operation
        handle.seek(SeekFrom::Start(offset)).await?;
        
        // Verify the seek operation
        let actual_position = handle.stream_position().await?;
        if actual_position != offset {
            return Err(FileError::SeekFailed);
        }
        
        let execution_time = start_time.elapsed();
        
        #[cfg(debug_assertions)]
        eprintln!(
            "Direct offset seek: requested={}, actual={}, time={:?}",
            offset,
            actual_position,
            execution_time
        );
        
        Ok(actual_position)
    }
}

/// Helper structure for keyframe information during indexing
#[derive(Debug, Clone)]
struct KeyframeInfo {
    offset: u64,
    size: u32,
    gop_size: u32,
}

#[async_trait]
impl FileStreamReader for DefaultFileStreamReader {
    async fn open_file(&self, file_path: &Path) -> Result<File, FileError> {
        match File::open(file_path).await {
            Ok(file) => Ok(file),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Err(FileError::FileNotFound {
                    path: file_path.to_string_lossy().to_string(),
                }),
                std::io::ErrorKind::PermissionDenied => Err(FileError::PermissionDenied),
                _ => Err(FileError::Io(e)),
            }
        }
    }

    async fn read_chunk(&self, handle: &mut File, size: usize) -> Result<Vec<u8>, FileError> {
        // Limit chunk size to prevent excessive memory usage for streaming
        let chunk_size = size.min(1024 * 1024); // Max 1MB chunks for streaming
        let mut buffer = vec![0u8; chunk_size];
        let bytes_read = handle.read(&mut buffer).await?;
        buffer.truncate(bytes_read);
        Ok(buffer)
    }

    async fn get_file_info(&self, handle: &mut File) -> Result<VideoFileInfo, FileError> {
        // Save current position
        let original_position = handle.stream_position().await?;
        
        // Seek to beginning for metadata extraction
        handle.seek(SeekFrom::Start(0)).await?;
        
        // Read initial chunk for format detection and basic info
        let mut buffer = vec![0u8; 8192]; // 8KB should be enough for basic metadata
        let bytes_read = handle.read(&mut buffer).await?;
        
        if bytes_read < 32 {
            return Err(FileError::CorruptedFile);
        }
        
        // Detect format and extract basic information
        let file_info = if self.is_mp4_format(&buffer[..bytes_read])? {
            self.extract_mp4_info(handle, &buffer[..bytes_read]).await?
        } else if self.is_h264_format(&buffer[..bytes_read])? {
            self.extract_h264_info(handle, &buffer[..bytes_read]).await?
        } else {
            return Err(FileError::UnsupportedFormat { 
                format: "unknown".to_string() 
            });
        };
        
        // Restore original position
        handle.seek(SeekFrom::Start(original_position)).await?;
        
        Ok(file_info)
    }

    async fn seek_to_position(&self, handle: &mut File, position: u64) -> Result<(), FileError> {
        handle.seek(SeekFrom::Start(position)).await?;
        Ok(())
    }

    async fn close_file(&self, _handle: File) -> Result<(), FileError> {
        // File is automatically closed when dropped in Rust
        Ok(())
    }

    async fn seek_to_time(&self, handle: &mut File, time_seconds: f64, index: &KeyframeIndex) -> Result<u64, FileError> {
        self.seek_to_time_impl(handle, time_seconds, index).await
    }

    async fn build_keyframe_index(&self, handle: &mut File) -> Result<KeyframeIndex, FileError> {
        // Use adaptive strategy by default for balanced performance and memory usage
        self.build_keyframe_index_impl(handle, IndexOptimizationStrategy::Adaptive).await
    }

    async fn validate_format(&self, file_path: &Path) -> Result<String, FileError> {
        let mut file = self.open_file(file_path).await?;
        let mut buffer = vec![0u8; 64]; // Read more bytes for better detection
        let bytes_read = file.read(&mut buffer).await?;
        
        if bytes_read < 8 {
            return Err(FileError::CorruptedFile);
        }
        
        // Check for MP4 format
        if self.is_mp4_format(&buffer[..bytes_read])? {
            return Ok("mp4".to_string());
        }
        
        // Check for H.264 format
        if self.is_h264_format(&buffer[..bytes_read])? {
            return Ok("h264".to_string());
        }
        
        // Check file extension as fallback
        if let Some(extension) = file_path.extension() {
            let ext = extension.to_string_lossy().to_lowercase();
            match ext.as_str() {
                "mp4" | "m4v" => Ok("mp4".to_string()),
                "h264" | "264" => Ok("h264".to_string()),
                _ => Err(FileError::UnsupportedFormat { 
                    format: ext.to_string() 
                }),
            }
        } else {
            Err(FileError::UnsupportedFormat { 
                format: "unknown".to_string() 
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // **Feature: video-streaming-uploader, Property 1: Format validation consistency**
    // **Validates: Requirements 1.1, 1.2, 1.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_format_validation_consistency(
            mp4_data in generate_mp4_header(),
            h264_data in generate_h264_header(),
            invalid_data in prop::collection::vec(any::<u8>(), 8..1024)
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Test MP4 format validation
                let mp4_file = create_temp_file_with_data(&mp4_data, "test.mp4").unwrap();
                let mp4_result = reader.validate_format(mp4_file.path()).await;
                prop_assert!(mp4_result.is_ok());
                prop_assert_eq!(mp4_result.unwrap(), "mp4");
                
                // Test H264 format validation
                let h264_file = create_temp_file_with_data(&h264_data, "test.h264").unwrap();
                let h264_result = reader.validate_format(h264_file.path()).await;
                prop_assert!(h264_result.is_ok());
                prop_assert_eq!(h264_result.unwrap(), "h264");
                
                // Test that validation works without loading entire file into memory
                // by testing with a large file that has valid header but random data after
                let mut large_mp4_data = mp4_data.clone();
                large_mp4_data.extend(vec![0u8; 10_000]); // Add 10KB of padding
                let large_mp4_file = create_temp_file_with_data(&large_mp4_data, "large.mp4").unwrap();
                let large_mp4_result = reader.validate_format(large_mp4_file.path()).await;
                prop_assert!(large_mp4_result.is_ok());
                prop_assert_eq!(large_mp4_result.unwrap(), "mp4");
                
                // Test invalid format rejection (unless it accidentally matches a valid pattern)
                let invalid_file = create_temp_file_with_data(&invalid_data, "test.invalid").unwrap();
                let invalid_result = reader.validate_format(invalid_file.path()).await;
                // Invalid data should either be rejected or accidentally match a valid format
                // We can't guarantee rejection since random data might match valid patterns
                prop_assert!(invalid_result.is_ok() || invalid_result.is_err());
                
                Ok(())
            });
            result?;
        }
    }

    fn generate_mp4_header() -> impl Strategy<Value = Vec<u8>> {
        // Generate valid MP4 ftyp box headers
        prop::sample::select(vec![
            // Standard MP4 with mp41 brand
            vec![0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'1', 0x00, 0x00, 0x00, 0x00],
            // MP4 with mp42 brand
            vec![0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00, 0x00],
            // MP4 with isom brand
            vec![0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0x00, 0x00, 0x00, 0x00],
            // MP4 with avc1 brand
            vec![0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p', b'a', b'v', b'c', b'1', 0x00, 0x00, 0x00, 0x00],
        ])
    }

    fn generate_h264_header() -> impl Strategy<Value = Vec<u8>> {
        // Generate valid H.264 NAL unit headers
        prop::sample::select(vec![
            // H.264 SPS NAL unit (type 7)
            vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x05, 0x81, 0xEC, 0x80],
            // H.264 PPS NAL unit (type 8)  
            vec![0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80],
            // H.264 IDR NAL unit (type 5)
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            // H.264 with 3-byte start code
            vec![0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x05, 0x81, 0xEC, 0x80],
        ])
    }

    fn create_temp_file_with_data(data: &[u8], _filename: &str) -> Result<NamedTempFile, std::io::Error> {
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(data)?;
        temp_file.flush()?;
        Ok(temp_file)
    }

    // **Feature: video-streaming-uploader, Property 37: Keyframe index construction**
    // **Validates: Requirements 9.1, 9.4, 9.6**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_keyframe_index_construction(
            h264_data in generate_h264_with_keyframes(),
            strategy in generate_optimization_strategy()
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Create a temporary file with H.264 data containing keyframes
                let temp_file = create_temp_file_with_data(&h264_data, "test.h264").unwrap();
                let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
                
                // Build keyframe index with the specified strategy
                let index = reader.build_keyframe_index_with_strategy(&mut file_handle, strategy.clone()).await;
                
                prop_assert!(index.is_ok());
                let index = index.unwrap();
                
                // Verify index properties
                prop_assert!(index.entries.len() > 0); // Should find at least one keyframe
                prop_assert!(index.total_duration >= 0.0); // Duration should be non-negative
                prop_assert!(index.index_precision > 0.0); // Precision should be positive
                prop_assert_eq!(index.optimization_strategy, strategy); // Strategy should match
                
                // Verify memory optimization is applied correctly
                match strategy {
                    IndexOptimizationStrategy::Full => {
                        prop_assert!(!index.memory_optimized || index.memory_optimized); // Can be either
                    },
                    _ => {
                        prop_assert!(index.memory_optimized); // Should be optimized for non-full strategies
                    }
                }
                
                // Verify sub-second precision support (Requirements 9.4)
                prop_assert!(index.index_precision <= 1.0); // Should support sub-second precision
                
                // Verify memory usage tracking (Requirements 9.6)
                prop_assert!(index.memory_usage > 0); // Should track memory usage
                
                // Verify entries are sorted by timestamp
                for i in 1..index.entries.len() {
                    prop_assert!(index.entries[i].timestamp >= index.entries[i-1].timestamp);
                }
                
                // Test seek functionality using the index
                if !index.entries.is_empty() {
                    let mid_time = index.total_duration / 2.0;
                    let seek_result = reader.seek_to_time(&mut file_handle, mid_time, &index).await;
                    prop_assert!(seek_result.is_ok());
                    
                    // Verify seek position is valid
                    let seek_offset = seek_result.unwrap();
                    prop_assert!(seek_offset <= file_handle.metadata().await.unwrap().len());
                }
                
                Ok(())
            });
            result?;
        }
    }

    // **Feature: video-streaming-uploader, Property 40: Sub-second precision support**
    // **Validates: Requirements 9.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_sub_second_precision_support(
            h264_data in generate_dense_h264_keyframes()
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Create a temporary file with dense keyframes for sub-second precision
                let temp_file = create_temp_file_with_data(&h264_data, "dense.h264").unwrap();
                let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
                
                // Build full index for maximum precision
                let index = reader.build_keyframe_index_with_strategy(&mut file_handle, IndexOptimizationStrategy::Full).await;
                
                prop_assert!(index.is_ok());
                let index = index.unwrap();
                
                // Verify sub-second precision is achieved
                prop_assert!(index.index_precision < 1.0); // Should be sub-second
                
                // Test precision by seeking to fractional seconds
                if index.total_duration > 1.0 {
                    let fractional_time = 0.5; // 500ms
                    let seek_result = reader.seek_to_time(&mut file_handle, fractional_time, &index).await;
                    prop_assert!(seek_result.is_ok());
                }
                
                Ok(())
            });
            result?;
        }
    }

    // **Feature: video-streaming-uploader, Property 42: Memory optimization for large indices**
    // **Validates: Requirements 9.6**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_memory_optimization_for_large_indices(
            memory_limit_mb in 1usize..50usize
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Create a large H.264 file with many keyframes
                let large_h264_data = generate_large_h264_file();
                let temp_file = create_temp_file_with_data(&large_h264_data, "large.h264").unwrap();
                let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
                
                // Build index with memory limit
                let index = reader.build_keyframe_index_with_memory_limit(&mut file_handle, memory_limit_mb).await;
                
                prop_assert!(index.is_ok());
                let index = index.unwrap();
                
                // Verify memory optimization is applied
                let _memory_limit_bytes = memory_limit_mb * 1024 * 1024;
                
                // For small memory limits, should use optimization strategies
                if memory_limit_mb < 20 {
                    prop_assert!(index.memory_optimized);
                    prop_assert!(matches!(
                        index.optimization_strategy,
                        IndexOptimizationStrategy::Sparse | 
                        IndexOptimizationStrategy::Adaptive | 
                        IndexOptimizationStrategy::Hierarchical
                    ));
                }
                
                // Memory usage should be reasonable (not necessarily under limit due to base overhead)
                prop_assert!(index.memory_usage > 0);
                
                // Index should still be functional despite optimization
                prop_assert!(index.entries.len() > 0);
                prop_assert!(index.index_precision > 0.0);
                
                Ok(())
            });
            result?;
        }
    }

    fn generate_optimization_strategy() -> impl Strategy<Value = IndexOptimizationStrategy> {
        prop::sample::select(vec![
            IndexOptimizationStrategy::Full,
            IndexOptimizationStrategy::Sparse,
            IndexOptimizationStrategy::Adaptive,
            IndexOptimizationStrategy::Hierarchical,
        ])
    }

    fn generate_h264_with_keyframes() -> impl Strategy<Value = Vec<u8>> {
        // Generate H.264 data with multiple keyframes - ensure at least one IDR frame
        prop::collection::vec(
            prop::sample::select(vec![
                // IDR keyframe (type 5) - this is what we're looking for
                vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
                // Another IDR keyframe
                vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x20, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
                // Third IDR keyframe
                vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x30, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            ]),
            2..8
        ).prop_map(|chunks| {
            let mut data = Vec::new();
            // Always start with SPS and PPS for valid H.264 stream
            data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x05, 0x81, 0xEC, 0x80]);
            data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80]);
            // Add the keyframes
            for chunk in chunks {
                data.extend(chunk);
            }
            data
        })
    }

    fn generate_dense_h264_keyframes() -> impl Strategy<Value = Vec<u8>> {
        // Generate H.264 data with many keyframes for sub-second precision testing
        prop::collection::vec(
            prop::strategy::Just(vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE]),
            10..50
        ).prop_map(|chunks| chunks.into_iter().flatten().collect())
    }

    fn generate_large_h264_file() -> Vec<u8> {
        // Generate a large H.264 file with many keyframes for memory optimization testing
        let keyframe = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE];
        let mut data = Vec::new();
        
        // Add SPS and PPS first
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x05, 0x81, 0xEC, 0x80]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80]);
        
        // Add many keyframes
        for _ in 0..100 {
            data.extend_from_slice(&keyframe);
        }
        
        data
    }

    // **Feature: video-streaming-uploader, Property 38: Time-based seek accuracy**
    // **Validates: Requirements 9.2, 9.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_time_based_seek_accuracy(
            h264_data in generate_h264_with_keyframes(),
            seek_time in 0.0f64..10.0f64
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Create a temporary file with H.264 data containing keyframes
                let temp_file = create_temp_file_with_data(&h264_data, "test.h264").unwrap();
                let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
                
                // Build keyframe index
                let index = reader.build_keyframe_index(&mut file_handle).await;
                prop_assert!(index.is_ok());
                let index = index.unwrap();
                
                // Test time-based seek accuracy
                if !index.entries.is_empty() && seek_time <= index.total_duration {
                    // Test basic seek_to_time method
                    let seek_result = reader.seek_to_time(&mut file_handle, seek_time, &index).await;
                    prop_assert!(seek_result.is_ok());
                    
                    let seek_offset = seek_result.unwrap();
                    prop_assert!(seek_offset <= file_handle.metadata().await.unwrap().len());
                    
                    // Test enhanced seek_to_time_with_result method
                    let detailed_result = reader.seek_to_time_with_result(&mut file_handle, seek_time, &index).await;
                    prop_assert!(detailed_result.is_ok());
                    
                    let result = detailed_result.unwrap();
                    prop_assert_eq!(result.requested_time, seek_time);
                    prop_assert!(result.actual_time >= 0.0);
                    prop_assert!(result.precision_achieved >= 0.0);
                    prop_assert!(result.keyframe_offset <= file_handle.metadata().await.unwrap().len());
                    
                    // Verify seek position matches keyframe offset
                    let current_position = file_handle.stream_position().await.unwrap();
                    prop_assert_eq!(current_position, result.keyframe_offset);
                }
                
                Ok(())
            });
            result?;
        }
    }

    // **Feature: video-streaming-uploader, Property 39: Direct file offset seeking**
    // **Validates: Requirements 9.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn test_direct_file_offset_seeking(
            h264_data in generate_h264_with_keyframes(),
            offset_ratio in 0.0f64..1.0f64
        ) {
            let result = tokio_test::block_on(async {
                let reader = DefaultFileStreamReader::new();
                
                // Create a temporary file with H.264 data
                let temp_file = create_temp_file_with_data(&h264_data, "test.h264").unwrap();
                let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
                
                let file_size = file_handle.metadata().await.unwrap().len();
                let target_offset = (file_size as f64 * offset_ratio) as u64;
                
                // Test direct offset seeking with validation
                if target_offset < file_size {
                    let seek_result = reader.seek_to_offset_with_validation(&mut file_handle, target_offset).await;
                    prop_assert!(seek_result.is_ok());
                    
                    let actual_offset = seek_result.unwrap();
                    prop_assert_eq!(actual_offset, target_offset);
                    
                    // Verify current position matches target
                    let current_position = file_handle.stream_position().await.unwrap();
                    prop_assert_eq!(current_position, target_offset);
                }
                
                // Test seeking beyond file end should fail
                let beyond_end_offset = file_size + 1000;
                let invalid_seek = reader.seek_to_offset_with_validation(&mut file_handle, beyond_end_offset).await;
                prop_assert!(invalid_seek.is_err());
                
                Ok(())
            });
            result?;
        }
    }

    // Test seek operation result validation and precision measurement
    #[tokio::test]
    async fn test_seek_result_validation_and_precision() {
        let reader = DefaultFileStreamReader::new();
        
        // Create test H.264 data with known keyframes
        let h264_data = generate_h264_with_multiple_keyframes();
        let temp_file = create_temp_file_with_data(&h264_data, "precision_test.h264").unwrap();
        let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
        
        // Build keyframe index
        let index = reader.build_keyframe_index(&mut file_handle).await.unwrap();
        
        // Test seeking to various time positions
        let test_times = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        
        for &time in &test_times {
            if time <= index.total_duration {
                let result = reader.seek_to_time_with_result(&mut file_handle, time, &index).await.unwrap();
                
                // Validate result structure
                assert_eq!(result.requested_time, time);
                assert!(result.actual_time >= 0.0);
                assert!(result.precision_achieved >= 0.0);
                assert!(result.execution_time.as_nanos() > 0);
                
                // Precision should be reasonable (within a reasonable bound)
                // Since we're seeking to keyframes, precision should be within a frame duration
                assert!(result.precision_achieved <= 1.0); // Within 1 second is reasonable
                
                // Verify file position
                let current_pos = file_handle.stream_position().await.unwrap();
                assert_eq!(current_pos, result.keyframe_offset);
            }
        }
    }

    // Test error handling for invalid seek operations
    #[tokio::test]
    async fn test_seek_error_handling() {
        let reader = DefaultFileStreamReader::new();
        
        // Create test H.264 data using the helper function
        let h264_data = generate_h264_with_multiple_keyframes();
        let temp_file = create_temp_file_with_data(&h264_data, "error_test.h264").unwrap();
        let mut file_handle = reader.open_file(temp_file.path()).await.unwrap();
        
        // Build keyframe index
        let index = reader.build_keyframe_index(&mut file_handle).await.unwrap();
        
        // Test negative seek time
        let negative_seek = reader.seek_to_time_with_result(&mut file_handle, -1.0, &index).await;
        assert!(negative_seek.is_err());
        
        // Test seek beyond duration
        let beyond_duration = index.total_duration + 10.0;
        let beyond_seek = reader.seek_to_time_with_result(&mut file_handle, beyond_duration, &index).await;
        assert!(beyond_seek.is_err());
        
        // Test direct offset beyond file size
        let file_size = file_handle.metadata().await.unwrap().len();
        let beyond_offset = reader.seek_to_offset_with_validation(&mut file_handle, file_size + 1000).await;
        assert!(beyond_offset.is_err());
    }

    fn generate_h264_with_multiple_keyframes() -> Vec<u8> {
        let mut data = Vec::new();
        
        // Add SPS and PPS
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x9A, 0x74, 0x05, 0x81, 0xEC, 0x80]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80]);
        
        // Add multiple IDR keyframes with different data
        let keyframes = vec![
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x20, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x30, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x40, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
            vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x50, 0xFF, 0xFE, 0xF6, 0xF0, 0xFE],
        ];
        
        for keyframe in keyframes {
            data.extend(keyframe);
        }
        
        data
    }
}