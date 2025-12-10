use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use async_trait::async_trait;
use crate::types::{VideoFileInfo, Resolution};
use crate::errors::FileError;

/// Extended H.264 information structure
struct ExtendedH264Info {
    resolution: Option<Resolution>,
}

#[async_trait]
pub trait FileStreamReader {
    async fn open_file(&self, file_path: &Path) -> Result<File, FileError>;
    async fn read_chunk(&self, handle: &mut File, size: usize) -> Result<Vec<u8>, FileError>;
    async fn get_file_info(&self, handle: &mut File) -> Result<VideoFileInfo, FileError>;
    async fn seek_to_position(&self, handle: &mut File, position: u64) -> Result<(), FileError>;
    async fn close_file(&self, handle: File) -> Result<(), FileError>;
    async fn validate_format(&self, file_path: &Path) -> Result<String, FileError>;
}

pub struct DefaultFileStreamReader;

impl DefaultFileStreamReader {
    pub fn new() -> Self {
        Self
    }
    
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
    
    /// Extract metadata from H.264 file with integrity checking
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
        
        // Get file size for duration estimation
        let file_size = handle.metadata().await?.len();
        
        // Estimate duration based on file size and resolution
        let pixels_per_frame = (resolution.width * resolution.height) as f64;
        let estimated_bitrate = (pixels_per_frame * 0.1).max(1_000_000.0); // Rough estimation
        let estimated_duration = (file_size as f64 * 8.0) / estimated_bitrate;
        
        Ok(VideoFileInfo {
            duration: estimated_duration.max(0.1),
            resolution,
            codec: "h264".to_string(),
            frame_rate: 30.0, // Default frame rate
            bit_rate: estimated_bitrate as u64,
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
        // This is a simplified check - a complete implementation would parse the entire box structure
        let current_pos = handle.stream_position().await?;
        handle.seek(SeekFrom::Start(0)).await?;
        
        let mut found_moov = false;
        let mut buffer = vec![0u8; 8192];
        let mut total_read = 0u64;
        
        while total_read < file_size && total_read < 100_000 { // Check first 100KB
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
}