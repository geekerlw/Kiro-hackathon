use crate::types::{AudioSegment, VideoSegment, SeparatedStreams, SyncInfo};
use crate::errors::SeparationError;
use tokio_stream::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use uuid::Uuid;

/// Trait for separating audio and video streams from mixed content
pub trait AudioVideoSeparator {
    /// Separate audio and video streams from input data
    async fn separate_streams(
        &self,
        input: impl Stream<Item = Vec<u8>> + Send + 'static,
    ) -> Result<SeparatedStreams, SeparationError>;
    
    /// Synchronize audio and video segments based on timestamps
    fn synchronize_streams(
        &self,
        video_segment: &VideoSegment,
        audio_segment: &AudioSegment,
    ) -> SyncInfo;
}

/// Default implementation of AudioVideoSeparator
#[derive(Debug, Clone)]
pub struct DefaultAudioVideoSeparator {
    /// Buffer size for processing chunks
    buffer_size: usize,
    /// Audio sample rate for processing
    default_sample_rate: u32,
    /// Default number of audio channels
    default_channels: u16,
}

impl DefaultAudioVideoSeparator {
    /// Create a new DefaultAudioVideoSeparator with default settings
    pub fn new() -> Self {
        Self {
            buffer_size: 8192,
            default_sample_rate: 44100,
            default_channels: 2,
        }
    }
    
    /// 公开方法：检测流类型
    pub fn detect_stream_types(&self, data: &[u8]) -> (bool, bool) {
        let has_video = data.windows(4).any(|window| {
            window == [0x00, 0x00, 0x00, 0x01] || window == [0x00, 0x00, 0x01, 0x00]
        });
        
        let has_audio = data.windows(2).any(|window| {
            window == [0xFF, 0xF1] || window == [0xFF, 0xF9]
        });
        
        (has_video, has_audio)
    }
    
    /// Create a new DefaultAudioVideoSeparator with custom settings
    pub fn with_config(buffer_size: usize, sample_rate: u32, channels: u16) -> Self {
        Self {
            buffer_size,
            default_sample_rate: sample_rate,
            default_channels: channels,
        }
    }
    
    /// Parse video data from mixed stream
    fn parse_video_data(&self, data: &[u8]) -> Result<VideoSegment, SeparationError> {
        // Simulate video parsing - in real implementation this would use FFmpeg or similar
        if data.len() < 4 {
            return Err(SeparationError::InvalidStreamFormat);
        }
        
        // Check for basic video markers (simplified)
        let has_video_marker = data.windows(4).any(|window| {
            // Look for common video frame markers (simplified)
            window == [0x00, 0x00, 0x00, 0x01] || // H.264 NAL unit start code
            window == [0x00, 0x00, 0x01, 0x00]    // Alternative marker
        });
        
        if !has_video_marker {
            return Err(SeparationError::InvalidStreamFormat);
        }
        
        // Extract video segment information
        let segment = VideoSegment {
            id: Uuid::new_v4(),
            data: data.to_vec(),
            timestamp: 0.0, // Would be extracted from stream in real implementation
            duration: 1.0 / 30.0, // Assume 30fps for now
            frame_count: 1,
            is_key_frame: data.len() > 1000, // Simplified key frame detection
            metadata: crate::types::SegmentMetadata {
                frame_indices: vec![0],
                key_frame_positions: if data.len() > 1000 { vec![0] } else { vec![] },
                encoding_params: std::collections::HashMap::new(),
            },
        };
        
        Ok(segment)
    }
    
    /// Parse audio data from mixed stream
    fn parse_audio_data(&self, data: &[u8]) -> Result<AudioSegment, SeparationError> {
        // Simulate audio parsing - in real implementation this would use FFmpeg or similar
        if data.len() < 2 {
            return Err(SeparationError::InvalidStreamFormat);
        }
        
        // Check for basic audio markers (simplified)
        let has_audio_marker = data.windows(2).any(|window| {
            // Look for common audio frame markers (simplified)
            window == [0xFF, 0xF1] || // AAC ADTS header
            window == [0xFF, 0xF9]    // Alternative AAC header
        });
        
        if !has_audio_marker {
            return Err(SeparationError::NoAudioTrack);
        }
        
        // Extract audio segment information
        let segment = AudioSegment {
            id: Uuid::new_v4(),
            data: data.to_vec(),
            timestamp: 0.0, // Would be extracted from stream in real implementation
            duration: 1024.0 / self.default_sample_rate as f64, // Assume 1024 samples per frame
            sample_rate: self.default_sample_rate,
            channels: self.default_channels,
        };
        
        Ok(segment)
    }
    

}

impl Default for DefaultAudioVideoSeparator {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioVideoSeparator for DefaultAudioVideoSeparator {
    async fn separate_streams(
        &self,
        input: impl Stream<Item = Vec<u8>> + Send + 'static,
    ) -> Result<SeparatedStreams, SeparationError> {
        // Create channels for separated streams
        let (video_tx, video_rx) = tokio::sync::mpsc::unbounded_channel();
        let (audio_tx, audio_rx) = tokio::sync::mpsc::unbounded_channel();
        
        let separator = self.clone();
        
        // Spawn task to process input stream and separate audio/video
        tokio::spawn(async move {
            let mut input = Box::pin(input);
            let mut video_timestamp = 0.0;
            let mut audio_timestamp = 0.0;
            
            while let Some(data) = input.next().await {
                let (has_video, has_audio) = separator.detect_stream_types(&data);
                
                // Process video data if present
                if has_video {
                    match separator.parse_video_data(&data) {
                        Ok(mut video_segment) => {
                            video_segment.timestamp = video_timestamp;
                            video_timestamp += video_segment.duration;
                            
                            if video_tx.send(video_segment).is_err() {
                                break; // Receiver dropped
                            }
                        }
                        Err(_) => {
                            // Skip invalid video data
                            continue;
                        }
                    }
                }
                
                // Process audio data if present
                if has_audio {
                    match separator.parse_audio_data(&data) {
                        Ok(mut audio_segment) => {
                            audio_segment.timestamp = audio_timestamp;
                            audio_timestamp += audio_segment.duration;
                            
                            if audio_tx.send(audio_segment).is_err() {
                                break; // Receiver dropped
                            }
                        }
                        Err(_) => {
                            // Skip invalid audio data
                            continue;
                        }
                    }
                }
                
                // If no valid streams found, return error
                if !has_video && !has_audio {
                    // For now, just continue - in real implementation might want to error
                    continue;
                }
            }
        });
        
        // Create stream wrappers
        let video_stream = VideoSegmentStream::new(video_rx);
        let audio_stream = AudioSegmentStream::new(audio_rx);
        
        Ok(SeparatedStreams {
            video_stream: Box::new(video_stream),
            audio_stream: Box::new(audio_stream),
        })
    }
    
    fn synchronize_streams(
        &self,
        video_segment: &VideoSegment,
        audio_segment: &AudioSegment,
    ) -> SyncInfo {
        let video_timestamp = video_segment.timestamp;
        let audio_timestamp = audio_segment.timestamp;
        let offset = video_timestamp - audio_timestamp;
        
        SyncInfo {
            video_timestamp,
            audio_timestamp,
            offset,
        }
    }
}

/// Stream wrapper for video segments
struct VideoSegmentStream {
    receiver: tokio::sync::mpsc::UnboundedReceiver<VideoSegment>,
}

impl VideoSegmentStream {
    fn new(receiver: tokio::sync::mpsc::UnboundedReceiver<VideoSegment>) -> Self {
        Self { receiver }
    }
}

impl Stream for VideoSegmentStream {
    type Item = VideoSegment;
    
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

impl Unpin for VideoSegmentStream {}

/// Stream wrapper for audio segments
struct AudioSegmentStream {
    receiver: tokio::sync::mpsc::UnboundedReceiver<AudioSegment>,
}

impl AudioSegmentStream {
    fn new(receiver: tokio::sync::mpsc::UnboundedReceiver<AudioSegment>) -> Self {
        Self { receiver }
    }
}

impl Stream for AudioSegmentStream {
    type Item = AudioSegment;
    
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

impl Unpin for AudioSegmentStream {}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::iter;

    #[tokio::test]
    async fn test_separator_creation() {
        let separator = DefaultAudioVideoSeparator::new();
        assert_eq!(separator.buffer_size, 8192);
        assert_eq!(separator.default_sample_rate, 44100);
        assert_eq!(separator.default_channels, 2);
    }

    #[tokio::test]
    async fn test_separator_with_config() {
        let separator = DefaultAudioVideoSeparator::with_config(4096, 48000, 1);
        assert_eq!(separator.buffer_size, 4096);
        assert_eq!(separator.default_sample_rate, 48000);
        assert_eq!(separator.default_channels, 1);
    }

    #[tokio::test]
    async fn test_detect_stream_types() {
        let separator = DefaultAudioVideoSeparator::new();
        
        // Test video data
        let video_data = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42]; // H.264 SPS NAL unit
        let (has_video, has_audio) = separator.detect_stream_types(&video_data);
        assert!(has_video);
        assert!(!has_audio);
        
        // Test audio data
        let audio_data = vec![0xFF, 0xF1, 0x50, 0x80]; // AAC ADTS header
        let (has_video, has_audio) = separator.detect_stream_types(&audio_data);
        assert!(!has_video);
        assert!(has_audio);
        
        // Test mixed data
        let mixed_data = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0xFF, 0xF1, 0x50];
        let (has_video, has_audio) = separator.detect_stream_types(&mixed_data);
        assert!(has_video);
        assert!(has_audio);
    }

    #[tokio::test]
    async fn test_parse_video_data() {
        let separator = DefaultAudioVideoSeparator::new();
        
        // Test valid video data
        let video_data = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
        let result = separator.parse_video_data(&video_data);
        assert!(result.is_ok());
        
        let segment = result.unwrap();
        assert_eq!(segment.data, video_data);
        assert_eq!(segment.frame_count, 1);
        assert!(!segment.is_key_frame); // Small data, not considered key frame
        
        // Test invalid video data
        let invalid_data = vec![0x01, 0x02];
        let result = separator.parse_video_data(&invalid_data);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_audio_data() {
        let separator = DefaultAudioVideoSeparator::new();
        
        // Test valid audio data
        let audio_data = vec![0xFF, 0xF1, 0x50, 0x80];
        let result = separator.parse_audio_data(&audio_data);
        assert!(result.is_ok());
        
        let segment = result.unwrap();
        assert_eq!(segment.data, audio_data);
        assert_eq!(segment.sample_rate, 44100);
        assert_eq!(segment.channels, 2);
        
        // Test invalid audio data
        let invalid_data = vec![0x01];
        let result = separator.parse_audio_data(&invalid_data);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_synchronize_streams() {
        let separator = DefaultAudioVideoSeparator::new();
        
        let video_segment = VideoSegment {
            id: Uuid::new_v4(),
            data: vec![1, 2, 3, 4],
            timestamp: 1.0,
            duration: 0.033,
            frame_count: 1,
            is_key_frame: true,
            metadata: crate::types::SegmentMetadata {
                frame_indices: vec![0],
                key_frame_positions: vec![0],
                encoding_params: std::collections::HashMap::new(),
            },
        };
        
        let audio_segment = AudioSegment {
            id: Uuid::new_v4(),
            data: vec![5, 6, 7, 8],
            timestamp: 0.9,
            duration: 0.023,
            sample_rate: 44100,
            channels: 2,
        };
        
        let sync_info = separator.synchronize_streams(&video_segment, &audio_segment);
        assert_eq!(sync_info.video_timestamp, 1.0);
        assert_eq!(sync_info.audio_timestamp, 0.9);
        assert!((sync_info.offset - 0.1).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_separate_streams_basic() {
        let separator = DefaultAudioVideoSeparator::new();
        
        // Create test data with both video and audio markers
        let test_data = vec![
            vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42], // Video data
            vec![0xFF, 0xF1, 0x50, 0x80, 0x12, 0x34], // Audio data
        ];
        
        let input_stream = iter(test_data);
        let result = separator.separate_streams(input_stream).await;
        assert!(result.is_ok());
        
        let _separated = result.unwrap();
        // Note: Testing the actual stream consumption would require more complex async testing
        // This test verifies that separation doesn't immediately fail
    }
}