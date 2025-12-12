use std::path::Path;
use std::process::Command;
use serde::{Deserialize, Serialize};
use tokio::process::Command as AsyncCommand;
use crate::errors::FFmpegError;

/// FFmpeg 命令行解析器
/// 使用 FFmpeg 命令行工具解析视频文件，生成关键帧时间轴数据
pub struct FFmpegCliParser {
    ffmpeg_path: String,
}

impl FFmpegCliParser {
    pub fn new() -> Self {
        Self {
            ffmpeg_path: "ffmpeg".to_string(),
        }
    }

    pub fn with_path(ffmpeg_path: String) -> Self {
        Self { ffmpeg_path }
    }

    /// 检查 FFmpeg 是否可用
    pub async fn check_ffmpeg_availability(&self) -> Result<FFmpegInfo, FFmpegError> {
        let output = AsyncCommand::new(&self.ffmpeg_path)
            .arg("-version")
            .output()
            .await
            .map_err(|e| FFmpegError::CommandFailed(format!("Failed to execute ffmpeg: {}", e)))?;

        if !output.status.success() {
            return Err(FFmpegError::NotAvailable);
        }

        let version_output = String::from_utf8_lossy(&output.stdout);
        let version = self.extract_version(&version_output)?;

        Ok(FFmpegInfo {
            version,
            available: true,
            supported_formats: vec!["mp4".to_string(), "h264".to_string(), "avi".to_string()],
        })
    }

    /// 解析视频文件，提取关键帧信息
    pub async fn parse_video_file(&self, file_path: &Path) -> Result<TimelineData, FFmpegError> {
        // 首先获取视频基本信息
        let video_info = self.get_video_info(file_path).await?;
        
        // 然后提取关键帧信息
        let keyframes = self.extract_keyframes(file_path).await?;

        // 计算文件校验和
        let checksum = self.calculate_file_checksum(file_path).await?;

        Ok(TimelineData {
            video_file_path: file_path.to_string_lossy().to_string(),
            video_file_size: tokio::fs::metadata(file_path).await
                .map_err(|e| FFmpegError::FileError(e.to_string()))?
                .len(),
            video_file_checksum: checksum,
            total_duration: video_info.duration,
            keyframes,
            generated_at: chrono::Utc::now().to_rfc3339(),
            ffmpeg_version: self.check_ffmpeg_availability().await?.version,
        })
    }

    /// 生成时间轴文件
    pub async fn generate_timeline_file(
        &self,
        video_path: &Path,
        timeline_data: &TimelineData,
    ) -> Result<(), FFmpegError> {
        let timeline_path = self.get_timeline_file_path(video_path);
        
        let json_data = serde_json::to_string_pretty(timeline_data)
            .map_err(|e| FFmpegError::SerializationError(e.to_string()))?;

        tokio::fs::write(&timeline_path, json_data)
            .await
            .map_err(|e| FFmpegError::FileError(format!("Failed to write timeline file: {}", e)))?;

        Ok(())
    }

    /// 加载时间轴文件
    pub async fn load_timeline_file(&self, video_path: &Path) -> Result<TimelineData, FFmpegError> {
        let timeline_path = self.get_timeline_file_path(video_path);
        
        let json_data = tokio::fs::read_to_string(&timeline_path)
            .await
            .map_err(|e| FFmpegError::FileError(format!("Failed to read timeline file: {}", e)))?;

        let timeline_data: TimelineData = serde_json::from_str(&json_data)
            .map_err(|e| FFmpegError::DeserializationError(e.to_string()))?;

        Ok(timeline_data)
    }

    /// 检查时间轴文件是否有效
    pub fn is_timeline_file_valid(&self, video_path: &Path, timeline_path: &Path) -> bool {
        // 检查时间轴文件是否存在
        if !timeline_path.exists() {
            return false;
        }

        // 检查视频文件的修改时间是否比时间轴文件新
        if let (Ok(video_meta), Ok(timeline_meta)) = (
            std::fs::metadata(video_path),
            std::fs::metadata(timeline_path),
        ) {
            if let (Ok(video_modified), Ok(timeline_modified)) = (
                video_meta.modified(),
                timeline_meta.modified(),
            ) {
                return timeline_modified >= video_modified;
            }
        }

        false
    }

    /// 获取时间轴文件路径
    fn get_timeline_file_path(&self, video_path: &Path) -> std::path::PathBuf {
        let mut timeline_path = video_path.to_path_buf();
        timeline_path.set_extension("timeline");
        timeline_path
    }

    /// 获取视频基本信息
    async fn get_video_info(&self, file_path: &Path) -> Result<VideoInfo, FFmpegError> {
        // 检查文件扩展名
        let extension = file_path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "h264" | "264" => {
                // 对于H.264原始流，使用特殊的处理方式
                self.get_h264_video_info(file_path).await
            }
            _ => {
                // 对于容器格式，使用标准方式
                self.get_container_video_info(file_path).await
            }
        }
    }

    /// 获取容器格式视频信息
    async fn get_container_video_info(&self, file_path: &Path) -> Result<VideoInfo, FFmpegError> {
        let output = AsyncCommand::new(&self.ffmpeg_path)
            .args(&[
                "-i",
                file_path.to_str().unwrap(),
                "-f",
                "null",
                "-",
            ])
            .output()
            .await
            .map_err(|e| FFmpegError::CommandFailed(format!("Failed to get video info: {}", e)))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        self.parse_video_info(&stderr)
    }

    /// 获取H.264原始流视频信息
    async fn get_h264_video_info(&self, file_path: &Path) -> Result<VideoInfo, FFmpegError> {
        // 对于H.264原始流，使用ffmpeg来获取更准确的信息
        let output = AsyncCommand::new(&self.ffmpeg_path)
            .args(&[
                "-i", file_path.to_str().unwrap(),
                "-f", "null",
                "-"
            ])
            .output()
            .await
            .map_err(|e| FFmpegError::CommandFailed(format!("Failed to get H.264 info: {}", e)))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        
        // 从ffmpeg输出中解析信息
        let mut width = 1280u32;
        let mut height = 720u32;
        let mut fps = 30.0f64;
        let mut frame_count = 0u32;
        let mut duration_seconds = 0.0f64;
        
        // 解析分辨率和帧率
        for line in stderr.lines() {
            if line.contains("Video:") && line.contains("fps") {
                // 解析分辨率 (例如: "1280x720")
                if let Some(res_match) = line.split_whitespace().find(|s| s.contains('x') && s.chars().next().unwrap_or('a').is_ascii_digit()) {
                    let parts: Vec<&str> = res_match.split('x').collect();
                    if parts.len() == 2 {
                        width = parts[0].parse().unwrap_or(1280);
                        height = parts[1].parse().unwrap_or(720);
                    }
                }
                
                // 解析帧率 (查找 "XX fps" 模式，但要小心选择正确的fps值)
                let words: Vec<&str> = line.split_whitespace().collect();
                for i in 0..words.len() {
                    if words[i] == "fps" && i > 0 {
                        if let Ok(parsed_fps) = words[i-1].parse::<f64>() {
                            // 对于H.264原始流，ffmpeg可能显示不准确的fps，我们需要从输出流获取
                            fps = parsed_fps;
                            break;
                        }
                    }
                }
            }
            
            // 解析输出流信息 (更准确的帧率)
            if line.contains("wrapped_avframe") && line.contains("fps") {
                let words: Vec<&str> = line.split_whitespace().collect();
                for i in 0..words.len() {
                    if words[i] == "fps," && i > 0 {
                        if let Ok(parsed_fps) = words[i-1].parse::<f64>() {
                            fps = parsed_fps; // 使用输出流的帧率，更准确
                            break;
                        }
                    }
                }
            }
            
            // 解析总帧数和时长 (例如: "frame= 1800 fps=0.0 q=-0.0 Lsize=N/A time=00:01:00.00")
            if line.contains("frame=") && line.contains("time=") {
                let words: Vec<&str> = line.split_whitespace().collect();
                for i in 0..words.len() {
                    if words[i].starts_with("frame=") {
                        let frame_str = words[i].trim_start_matches("frame=");
                        frame_count = frame_str.parse().unwrap_or(0);
                    }
                    if words[i].starts_with("time=") {
                        let time_str = words[i].trim_start_matches("time=");
                        // 解析时间格式 HH:MM:SS.ss
                        if let Ok(parsed_duration) = self.parse_time_to_seconds(time_str) {
                            duration_seconds = parsed_duration;
                        }
                    }
                }
            }
        }
        
        // 计算时长 - 优先使用ffmpeg直接报告的时长
        let duration = if duration_seconds > 0.0 {
            duration_seconds
        } else if fps > 0.0 && frame_count > 0 {
            frame_count as f64 / fps
        } else {
            0.0
        };

        Ok(VideoInfo {
            duration,
            width,
            height,
            fps,
        })
    }

    /// 提取关键帧信息
    async fn extract_keyframes(&self, file_path: &Path) -> Result<Vec<KeyframeInfo>, FFmpegError> {
        // 检查文件扩展名来决定处理方式
        let extension = file_path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "h264" | "264" => {
                // 对于原始H.264文件，使用特殊的处理方式
                self.extract_h264_keyframes(file_path).await
            }
            _ => {
                // 对于容器格式（MP4等），使用标准方式
                self.extract_container_keyframes(file_path).await
            }
        }
    }

    /// 提取容器格式文件的关键帧信息
    async fn extract_container_keyframes(&self, file_path: &Path) -> Result<Vec<KeyframeInfo>, FFmpegError> {
        let output = AsyncCommand::new("ffprobe")
            .args(&[
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "packet=pos,pts_time,flags,size",
                "-of", "csv=p=0",
                file_path.to_str().unwrap(),
            ])
            .output()
            .await
            .map_err(|e| FFmpegError::CommandFailed(format!("Failed to extract keyframes: {}", e)))?;

        if !output.status.success() {
            return Err(FFmpegError::CommandFailed("ffprobe command failed".to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_keyframes_from_ffprobe(&stdout)
    }

    /// 提取H.264原始流的关键帧信息
    async fn extract_h264_keyframes(&self, file_path: &Path) -> Result<Vec<KeyframeInfo>, FFmpegError> {
        // 首先获取视频信息以确定帧率
        let video_info = self.get_video_info(file_path).await?;
        
        // 使用ffprobe获取帧信息，但不依赖pts_time
        let output = AsyncCommand::new("ffprobe")
            .args(&[
                "-v", "quiet",
                "-select_streams", "v:0",
                "-show_entries", "packet=pos,flags,size",
                "-of", "csv=p=0",
                file_path.to_str().unwrap(),
            ])
            .output()
            .await
            .map_err(|e| FFmpegError::CommandFailed(format!("Failed to extract H.264 keyframes: {}", e)))?;

        if !output.status.success() {
            return Err(FFmpegError::CommandFailed("ffprobe command failed for H.264".to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_h264_keyframes_from_ffprobe(&stdout, video_info.fps)
    }

    /// 解析视频信息
    fn parse_video_info(&self, ffmpeg_output: &str) -> Result<VideoInfo, FFmpegError> {
        let duration = self.extract_duration(ffmpeg_output)?;
        
        // 解析分辨率和帧率
        let mut width = 1280;  // 默认值
        let mut height = 720;  // 默认值
        let mut fps = 30.0;    // 默认值
        
        for line in ffmpeg_output.lines() {
            if line.contains("Video:") {
                // 查找分辨率信息，格式类似: 1280x720
                if let Some(resolution_match) = line.split_whitespace()
                    .find(|part| part.contains('x') && part.chars().next().unwrap_or('a').is_ascii_digit()) {
                    let res_parts: Vec<&str> = resolution_match.split('x').collect();
                    if res_parts.len() == 2 {
                        if let (Ok(w), Ok(h)) = (res_parts[0].parse::<u32>(), res_parts[1].parse::<u32>()) {
                            width = w;
                            height = h;
                        }
                    }
                }
                
                // 查找帧率信息，格式类似: 29.97 fps
                if let Some(fps_pos) = line.find(" fps") {
                    let before_fps = &line[..fps_pos];
                    if let Some(last_space) = before_fps.rfind(' ') {
                        let fps_str = &before_fps[last_space + 1..];
                        if let Ok(parsed_fps) = fps_str.parse::<f64>() {
                            fps = parsed_fps;
                        }
                    }
                }
            }
        }
        
        Ok(VideoInfo {
            duration,
            width,
            height,
            fps,
        })
    }

    /// 解析 ffprobe 输出的关键帧信息（容器格式）
    fn parse_keyframes_from_ffprobe(&self, ffprobe_output: &str) -> Result<Vec<KeyframeInfo>, FFmpegError> {
        let mut keyframes = Vec::new();
        
        for line in ffprobe_output.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 4 {
                // ffprobe 输出格式: pos,pts_time,flags,size
                let pos_str = parts[0];
                let pts_time_str = parts[1];
                let flags_str = parts[2];
                let size_str = parts[3];
                
                // 只处理关键帧 (flags 包含 "K")
                if flags_str.contains('K') {
                    if let (Ok(timestamp), Ok(file_offset), Ok(frame_size)) = (
                        pts_time_str.parse::<f64>(),
                        pos_str.parse::<u64>(),
                        size_str.parse::<u32>(),
                    ) {
                        keyframes.push(KeyframeInfo {
                            timestamp,
                            file_offset,
                            frame_size,
                            pts: (timestamp * 30.0) as i64, // 假设30fps，实际应该从视频信息中获取
                            dts: (timestamp * 30.0) as i64,
                        });
                    }
                }
            }
        }
        
        if keyframes.is_empty() {
            return Err(FFmpegError::ParseError("No keyframes found in video".to_string()));
        }
        
        // 按时间戳排序
        keyframes.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
        
        Ok(keyframes)
    }

    /// 解析 H.264 原始流的关键帧信息
    fn parse_h264_keyframes_from_ffprobe(&self, ffprobe_output: &str, fps: f64) -> Result<Vec<KeyframeInfo>, FFmpegError> {
        let mut keyframes = Vec::new();
        let mut frame_count = 0;
        
        for line in ffprobe_output.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                // ffprobe 输出格式: size,pos,flags
                let size_str = parts[0];
                let pos_str = parts[1];
                let flags_str = parts[2];
                
                // 只处理关键帧 (flags 包含 "K")
                if flags_str.contains('K') {
                    if let (Ok(file_offset), Ok(frame_size)) = (
                        pos_str.parse::<u64>(),
                        size_str.parse::<u32>(),
                    ) {
                        // 对于H.264原始流，根据帧计数和帧率计算时间戳
                        let timestamp = frame_count as f64 / fps;
                        
                        keyframes.push(KeyframeInfo {
                            timestamp,
                            file_offset,
                            frame_size,
                            pts: (timestamp * fps) as i64,
                            dts: (timestamp * fps) as i64,
                        });
                    }
                }
                frame_count += 1;
            }
        }
        
        if keyframes.is_empty() {
            return Err(FFmpegError::ParseError("No keyframes found in H.264 stream".to_string()));
        }
        
        // 按时间戳排序
        keyframes.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
        
        Ok(keyframes)
    }

    /// 提取版本信息
    fn extract_version(&self, version_output: &str) -> Result<String, FFmpegError> {
        // 从 FFmpeg 版本输出中提取版本号
        if let Some(line) = version_output.lines().next() {
            if line.starts_with("ffmpeg version") {
                return Ok(line.to_string());
            }
        }
        Err(FFmpegError::ParseError("Could not parse FFmpeg version".to_string()))
    }

    /// 提取时长信息
    fn extract_duration(&self, ffmpeg_output: &str) -> Result<f64, FFmpegError> {
        // 从 FFmpeg 输出中提取视频时长
        // 查找类似 "Duration: 00:01:30.45" 的行
        for line in ffmpeg_output.lines() {
            if line.contains("Duration:") {
                // 查找 Duration: 后面的时间格式 HH:MM:SS.ss
                if let Some(duration_start) = line.find("Duration: ") {
                    let duration_part = &line[duration_start + 10..];
                    if let Some(comma_pos) = duration_part.find(',') {
                        let time_str = &duration_part[..comma_pos];
                        return self.parse_time_to_seconds(time_str);
                    }
                }
            }
        }
        Err(FFmpegError::ParseError("Could not parse video duration".to_string()))
    }
    
    /// 将时间字符串 (HH:MM:SS.ss) 转换为秒数
    fn parse_time_to_seconds(&self, time_str: &str) -> Result<f64, FFmpegError> {
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 3 {
            return Err(FFmpegError::ParseError(format!("Invalid time format: {}", time_str)));
        }
        
        let hours: f64 = parts[0].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid hours: {}", parts[0])))?;
        let minutes: f64 = parts[1].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid minutes: {}", parts[1])))?;
        let seconds: f64 = parts[2].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid seconds: {}", parts[2])))?;
        
        Ok(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    /// 解析H.264视频信息
    fn parse_h264_video_info(&self, ffprobe_output: &str) -> Result<VideoInfo, FFmpegError> {
        // ffprobe输出格式: width,height,r_frame_rate,nb_read_frames
        let line = ffprobe_output.lines().next()
            .ok_or_else(|| FFmpegError::ParseError("No output from ffprobe".to_string()))?;
        
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 {
            return Err(FFmpegError::ParseError(format!("Invalid ffprobe output format: {}", line)));
        }

        // 解析分辨率
        let width: u32 = parts[0].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid width: {}", parts[0])))?;
        let height: u32 = parts[1].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid height: {}", parts[1])))?;

        // 解析帧率 (格式可能是 "120/1" 或 "60")
        let fps_str = parts[2];
        let fps = if fps_str.contains('/') {
            let fps_parts: Vec<&str> = fps_str.split('/').collect();
            if fps_parts.len() == 2 {
                let numerator: f64 = fps_parts[0].parse()
                    .map_err(|_| FFmpegError::ParseError(format!("Invalid fps numerator: {}", fps_parts[0])))?;
                let denominator: f64 = fps_parts[1].parse()
                    .map_err(|_| FFmpegError::ParseError(format!("Invalid fps denominator: {}", fps_parts[1])))?;
                numerator / denominator
            } else {
                60.0 // 默认值
            }
        } else {
            fps_str.parse::<f64>().unwrap_or(60.0)
        };

        // 解析帧数
        let frame_count: u32 = parts[3].parse()
            .map_err(|_| FFmpegError::ParseError(format!("Invalid frame count: {}", parts[3])))?;

        // 计算时长
        let duration = if fps > 0.0 {
            frame_count as f64 / fps
        } else {
            0.0
        };

        Ok(VideoInfo {
            duration,
            width,
            height,
            fps,
        })
    }

    /// 计算文件校验和
    async fn calculate_file_checksum(&self, file_path: &Path) -> Result<String, FFmpegError> {
        // 简化实现：使用文件大小和修改时间作为校验和
        let metadata = tokio::fs::metadata(file_path)
            .await
            .map_err(|e| FFmpegError::FileError(e.to_string()))?;
        
        Ok(format!("{}_{}", metadata.len(), 
            metadata.modified()
                .map_err(|e| FFmpegError::FileError(e.to_string()))?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()))
    }
}

/// FFmpeg 信息
#[derive(Debug, Clone)]
pub struct FFmpegInfo {
    pub version: String,
    pub available: bool,
    pub supported_formats: Vec<String>,
}

/// 时间轴数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineData {
    pub video_file_path: String,
    pub video_file_size: u64,
    pub video_file_checksum: String,
    pub total_duration: f64,
    pub keyframes: Vec<KeyframeInfo>,
    pub generated_at: String,
    pub ffmpeg_version: String,
}

/// 关键帧信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeInfo {
    pub timestamp: f64,        // 时间戳（秒）
    pub file_offset: u64,      // 文件偏移位置（字节）
    pub frame_size: u32,       // 关键帧大小（字节）
    pub pts: i64,              // 显示时间戳
    pub dts: i64,              // 解码时间戳
}

/// 视频信息
#[derive(Debug, Clone)]
struct VideoInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_ffmpeg_availability() {
        let parser = FFmpegCliParser::new();
        
        // 这个测试可能会失败如果系统没有安装 FFmpeg
        // 在实际应用中，应该优雅地处理这种情况
        match parser.check_ffmpeg_availability().await {
            Ok(info) => {
                assert!(info.available);
                assert!(!info.version.is_empty());
            }
            Err(FFmpegError::NotAvailable) => {
                println!("FFmpeg not available on this system - test skipped");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_timeline_file_path() {
        let parser = FFmpegCliParser::new();
        let video_path = PathBuf::from("/path/to/video.mp4");
        let timeline_path = parser.get_timeline_file_path(&video_path);
        
        assert_eq!(timeline_path, PathBuf::from("/path/to/video.timeline"));
    }

    #[test]
    fn test_version_extraction() {
        let parser = FFmpegCliParser::new();
        let version_output = "ffmpeg version 4.4.0 Copyright (c) 2000-2021 the FFmpeg developers";
        
        let version = parser.extract_version(version_output).unwrap();
        assert!(version.contains("ffmpeg version 4.4.0"));
    }

    #[test]
    fn test_timeline_data_serialization() {
        use serde_json;
        
        let timeline = TimelineData {
            video_file_path: "/test/video.mp4".to_string(),
            video_file_size: 1024000,
            video_file_checksum: "test_checksum".to_string(),
            total_duration: 120.5,
            keyframes: vec![
                KeyframeInfo {
                    timestamp: 0.0,
                    file_offset: 0,
                    frame_size: 50000,
                    pts: 0,
                    dts: 0,
                },
                KeyframeInfo {
                    timestamp: 2.0,
                    file_offset: 100000,
                    frame_size: 48000,
                    pts: 60,
                    dts: 60,
                },
            ],
            generated_at: "2023-12-01T10:30:00Z".to_string(),
            ffmpeg_version: "ffmpeg version 4.4.0".to_string(),
        };
        
        // 测试序列化
        let json = serde_json::to_string(&timeline).unwrap();
        assert!(json.contains("video.mp4"));
        assert!(json.contains("120.5"));
        
        // 测试反序列化
        let deserialized: TimelineData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_duration, 120.5);
        assert_eq!(deserialized.keyframes.len(), 2);
    }
}