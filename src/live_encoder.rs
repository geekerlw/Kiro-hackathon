use std::sync::Arc;
use std::time::{SystemTime, Instant};
use std::process::{Stdio};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::types::{VideoSegment, Segment, SegmentMetadata};
use crate::errors::TransportError;
use crate::mock_server::LiveStreamQuality;

/// FFmpeg实时H.264编码器
/// 使用FFmpeg录制屏幕并生成H.264流
pub struct LiveH264Encoder {
    /// 编码器配置
    config: LiveEncoderConfig,
    /// 编码状态
    encoding_state: Arc<RwLock<EncodingState>>,
    /// 输出数据发送器
    output_sender: Option<mpsc::Sender<Vec<u8>>>,
    /// 输出数据接收器
    output_receiver: Option<mpsc::Receiver<Vec<u8>>>,
    /// 时间戳生成器
    timestamp_generator: TimestampGenerator,
    /// 性能统计
    stats: Arc<Mutex<EncodingStats>>,
    /// 编码任务句柄
    encoding_task: Option<tokio::task::JoinHandle<()>>,
    /// FFmpeg进程
    ffmpeg_process: Option<tokio::process::Child>,
}

/// 编码器配置
#[derive(Debug, Clone)]
pub struct LiveEncoderConfig {
    pub quality: LiveStreamQuality,
    pub timestamp_overlay: bool,
    pub screen_capture: bool,
    pub output_format: OutputFormat,
    pub segment_duration_ms: u64,
    pub timestamp_format: TimestampFormat,
}

/// 时间戳显示格式
#[derive(Debug, Clone)]
pub enum TimestampFormat {
    /// 毫秒时间戳 (更精确，便于延迟测试)
    Milliseconds,
    /// 标准时间格式
    DateTime,
    /// 帧计数器
    FrameCounter,
    /// 组合显示 (时间戳 + 帧数)
    Combined,
}

/// 输出格式
#[derive(Debug, Clone)]
pub enum OutputFormat {
    H264Raw,
    H264Annex,
    FragmentedMP4,
    TestPattern, // 纯Rust测试图案
}

/// 编码状态
#[derive(Debug, Clone)]
pub struct EncodingState {
    pub is_encoding: bool,
    pub stream_id: String,
    pub start_time: Option<SystemTime>,
    pub frames_encoded: u64,
    pub bytes_encoded: u64,
    pub current_timestamp_ms: u64,
}

/// 时间戳生成器
pub struct TimestampGenerator {
    start_time: Instant,
    frame_count: u64,
    fps: u32,
}

/// 编码性能统计
#[derive(Debug, Clone)]
pub struct EncodingStats {
    pub encoding_fps: f64,
    pub bitrate_kbps: f64,
    pub frame_drops: u64,
    pub avg_encode_time_ms: f64,
    pub peak_encode_time_ms: f64,
}

impl LiveH264Encoder {
    /// 创建新的实时编码器
    pub fn new(config: LiveEncoderConfig) -> Self {
        Self {
            config: config.clone(),
            encoding_state: Arc::new(RwLock::new(EncodingState {
                is_encoding: false,
                stream_id: String::new(),
                start_time: None,
                frames_encoded: 0,
                bytes_encoded: 0,
                current_timestamp_ms: 0,
            })),
            output_sender: None,
            output_receiver: None,
            timestamp_generator: TimestampGenerator::new(config.quality.fps),
            stats: Arc::new(Mutex::new(EncodingStats {
                encoding_fps: 0.0,
                bitrate_kbps: 0.0,
                frame_drops: 0,
                avg_encode_time_ms: 0.0,
                peak_encode_time_ms: 0.0,
            })),
            encoding_task: None,
            ffmpeg_process: None,
        }
    }

    /// 开始实时编码
    pub async fn start_encoding(&mut self, stream_id: String) -> Result<(), TransportError> {
        info!("Starting FFmpeg live encoding for stream: {}", stream_id);

        // 更新编码状态
        {
            let mut state = self.encoding_state.write().await;
            state.is_encoding = true;
            state.stream_id = stream_id.clone();
            state.start_time = Some(SystemTime::now());
            state.frames_encoded = 0;
            state.bytes_encoded = 0;
        }

        // 创建输出通道
        let (tx, rx) = mpsc::channel(1000); // 增大缓冲区以处理H.264流
        self.output_sender = Some(tx);
        self.output_receiver = Some(rx);

        // 启动编码任务
        let encoding_state = self.encoding_state.clone();
        let stats = self.stats.clone();
        let config = self.config.clone();
        let sender = self.output_sender.as_ref().unwrap().clone();

        let stream_id_clone = stream_id.clone();
        let task = tokio::spawn(async move {
            if let Err(e) = Self::ffmpeg_encoding_loop(
                stream_id_clone,
                config,
                sender,
                encoding_state,
                stats,
            ).await {
                error!("FFmpeg encoding loop failed: {}", e);
            }
        });

        self.encoding_task = Some(task);

        // 重置时间戳生成器
        self.timestamp_generator.reset();

        info!("FFmpeg live encoding started successfully for stream: {}", stream_id);
        Ok(())
    }

    /// 停止实时编码
    pub async fn stop_encoding(&mut self) -> Result<(), TransportError> {
        info!("Stopping FFmpeg live encoding");

        // 更新编码状态
        {
            let mut state = self.encoding_state.write().await;
            state.is_encoding = false;
        }

        // 终止FFmpeg进程
        if let Some(mut process) = self.ffmpeg_process.take() {
            if let Err(e) = process.kill().await {
                warn!("Failed to kill FFmpeg process: {}", e);
            }
            info!("FFmpeg process terminated");
        }

        // 终止编码任务
        if let Some(task) = self.encoding_task.take() {
            task.abort();
            info!("Encoding task terminated");
        }

        // 清理通道
        self.output_sender = None;
        self.output_receiver = None;

        info!("FFmpeg live encoding stopped");
        Ok(())
    }

    /// 获取下一个编码分片
    pub async fn get_next_segment(&mut self) -> Option<Segment> {
        if let Some(receiver) = &mut self.output_receiver {
            if let Ok(data) = receiver.try_recv() {
                return Some(self.create_video_segment(data).await);
            }
        }
        None
    }

    /// 获取当前编码状态
    pub async fn get_encoding_state(&self) -> EncodingState {
        self.encoding_state.read().await.clone()
    }

    /// 获取编码统计信息
    pub async fn get_stats(&self) -> EncodingStats {
        self.stats.lock().await.clone()
    }

    /// FFmpeg编码循环
    async fn ffmpeg_encoding_loop(
        stream_id: String,
        config: LiveEncoderConfig,
        sender: mpsc::Sender<Vec<u8>>,
        encoding_state: Arc<RwLock<EncodingState>>,
        stats: Arc<Mutex<EncodingStats>>,
    ) -> Result<(), TransportError> {
        info!("Starting FFmpeg encoding loop for stream: {}", stream_id);
        
        // 构建FFmpeg命令
        let mut ffmpeg_cmd = TokioCommand::new("ffmpeg");
        
        // 输入配置 - 使用avfoundation录制屏幕
        ffmpeg_cmd
            .arg("-f").arg("avfoundation")
            .arg("-i").arg("4") // macOS屏幕序号4
            .arg("-r").arg(config.quality.fps.to_string()) // 帧率
            .arg("-s").arg(format!("{}x{}", config.quality.width, config.quality.height)) // 分辨率
            
            // H.264编码配置
            .arg("-c:v").arg("libx264")
            .arg("-preset").arg("ultrafast") // 最快编码速度
            .arg("-tune").arg("zerolatency") // 零延迟调优
            .arg("-profile:v").arg("baseline") // Baseline Profile
            .arg("-level").arg("3.1") // Level 3.1
            .arg("-pix_fmt").arg("yuv420p") // 像素格式
            .arg("-b:v").arg(format!("{}k", config.quality.bitrate_kbps)) // 码率
            .arg("-g").arg(config.quality.keyframe_interval.to_string()) // GOP大小
            
            // 时间戳叠加 (如果启用)
            .arg("-vf").arg(if config.timestamp_overlay {
                format!("drawtext=text='%{{pts\\:hms}} | Frame\\: %{{n}} | {}x{} | {}fps':fontcolor=yellow:fontsize=24:box=1:boxcolor=black@0.5:x=10:y=10", 
                    config.quality.width, config.quality.height, config.quality.fps)
            } else {
                "null".to_string()
            })
            
            // 输出配置
            .arg("-f").arg("h264") // 输出格式为原始H.264
            .arg("-") // 输出到stdout
            
            // 其他选项
            .arg("-y") // 覆盖输出文件
            .arg("-loglevel").arg("error"); // 只显示错误日志

        // 配置进程
        ffmpeg_cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        info!("Starting FFmpeg process with command: {:?}", ffmpeg_cmd);
        
        // 启动FFmpeg进程
        let mut child = ffmpeg_cmd.spawn()
            .map_err(|e| TransportError::ConfigurationError { 
                message: format!("Failed to start FFmpeg: {}. Make sure FFmpeg is installed and avfoundation is available.", e) 
            })?;

        let stdout = child.stdout.take()
            .ok_or_else(|| TransportError::ConfigurationError { 
                message: "Failed to get FFmpeg stdout".to_string() 
            })?;

        let stderr = child.stderr.take()
            .ok_or_else(|| TransportError::ConfigurationError { 
                message: "Failed to get FFmpeg stderr".to_string() 
            })?;

        // 启动错误日志监控
        let stream_id_clone = stream_id.clone();
        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut buffer = Vec::new();
            while let Ok(n) = stderr_reader.read_buf(&mut buffer).await {
                if n == 0 { break; }
                let stderr_output = String::from_utf8_lossy(&buffer);
                if !stderr_output.trim().is_empty() {
                    warn!("FFmpeg stderr ({}): {}", stream_id_clone, stderr_output.trim());
                }
                buffer.clear();
            }
        });

        // 读取H.264数据流
        let mut stdout_reader = BufReader::new(stdout);
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB缓冲区
        let mut frame_count = 0u64;
        let start_time = Instant::now();

        loop {
            // 检查是否应该停止编码
            {
                let state = encoding_state.read().await;
                if !state.is_encoding {
                    break;
                }
            }

            // 读取数据
            match stdout_reader.read(&mut buffer).await {
                Ok(0) => {
                    info!("FFmpeg process ended (EOF)");
                    break;
                }
                Ok(n) => {
                    let data = buffer[..n].to_vec();
                    
                    // 更新统计信息
                    frame_count += 1;
                    {
                        let mut state = encoding_state.write().await;
                        state.frames_encoded = frame_count;
                        state.bytes_encoded += n as u64;
                        
                        if let Some(start_time) = state.start_time {
                            if let Ok(elapsed) = SystemTime::now().duration_since(start_time) {
                                state.current_timestamp_ms = elapsed.as_millis() as u64;
                            }
                        }
                    }

                    // 更新性能统计
                    {
                        let mut stats_guard = stats.lock().await;
                        let elapsed_secs = start_time.elapsed().as_secs_f64();
                        if elapsed_secs > 0.0 {
                            stats_guard.encoding_fps = frame_count as f64 / elapsed_secs;
                            stats_guard.bitrate_kbps = (frame_count * n as u64 * 8) as f64 / elapsed_secs / 1000.0;
                        }
                    }

                    // 发送数据
                    if let Err(e) = sender.send(data).await {
                        error!("Failed to send H.264 data: {}", e);
                        break;
                    }

                    // 每100帧打印一次统计
                    if frame_count % 100 == 0 {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let fps = frame_count as f64 / elapsed;
                        info!("FFmpeg encoding: {} chunks, {:.1} fps, {:.1}s elapsed", 
                              frame_count, fps, elapsed);
                    }
                }
                Err(e) => {
                    error!("Failed to read from FFmpeg stdout: {}", e);
                    break;
                }
            }
        }

        // 等待进程结束
        match child.wait().await {
            Ok(status) => {
                if status.success() {
                    info!("FFmpeg process completed successfully");
                } else {
                    warn!("FFmpeg process exited with status: {}", status);
                }
            }
            Err(e) => {
                error!("Failed to wait for FFmpeg process: {}", e);
            }
        }

        info!("FFmpeg encoding loop ended for stream: {}", stream_id);
        Ok(())
    }

    /// 创建视频分片
    async fn create_video_segment(&mut self, data: Vec<u8>) -> Segment {
        let timestamp = self.timestamp_generator.next_timestamp();
        let current_ms = self.timestamp_generator.current_timestamp_ms();
        
        // 打印时间戳用于延迟对比
        println!("🎬 LIVE ENCODE: {}ms - Chunk {} ({} bytes)", 
                current_ms, 
                self.timestamp_generator.frame_count, 
                data.len());

        let state = self.encoding_state.read().await;
        
        Segment::Video(VideoSegment {
            id: Uuid::new_v4(),
            data,
            timestamp,
            duration: 1.0 / self.config.quality.fps as f64,
            frame_count: 1,
            is_key_frame: self.timestamp_generator.frame_count % self.config.quality.keyframe_interval as u64 == 1,
            metadata: SegmentMetadata {
                frame_indices: vec![self.timestamp_generator.frame_count as usize],
                key_frame_positions: vec![],
                encoding_params: {
                    let mut params = std::collections::HashMap::new();
                    params.insert("live_stream".to_string(), "true".to_string());
                    params.insert("stream_id".to_string(), state.stream_id.clone());
                    params.insert("timestamp_ms".to_string(), current_ms.to_string());
                    params.insert("frame_number".to_string(), self.timestamp_generator.frame_count.to_string());
                    params.insert("encoding_fps".to_string(), self.config.quality.fps.to_string());
                    params.insert("bitrate_kbps".to_string(), self.config.quality.bitrate_kbps.to_string());
                    params.insert("encoder".to_string(), "ffmpeg".to_string());
                    params.insert("input_source".to_string(), "avfoundation:4".to_string());
                    params
                },
            },
        })
    }
}

impl TimestampGenerator {
    pub fn new(fps: u32) -> Self {
        Self {
            start_time: Instant::now(),
            frame_count: 0,
            fps,
        }
    }

    pub fn reset(&mut self) {
        self.start_time = Instant::now();
        self.frame_count = 0;
    }

    pub fn next_timestamp(&mut self) -> f64 {
        self.frame_count += 1;
        (self.frame_count - 1) as f64 / self.fps as f64
    }

    pub fn current_timestamp_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

impl Default for LiveEncoderConfig {
    fn default() -> Self {
        Self {
            quality: LiveStreamQuality {
                width: 1280,
                height: 720,
                fps: 30,
                bitrate_kbps: 2000,
                keyframe_interval: 30, // 1 second at 30fps
            },
            timestamp_overlay: true,
            screen_capture: true, // 默认使用屏幕录制
            output_format: OutputFormat::H264Raw, // 使用标准H.264格式
            segment_duration_ms: 33, // ~30fps
            timestamp_format: TimestampFormat::Combined, // 组合显示，便于延迟测试
        }
    }
}