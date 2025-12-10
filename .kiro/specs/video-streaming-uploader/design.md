# Design Document

## Overview

Video Streaming Uploader是一个高性能的视频流上传系统，专为极低延迟传输而设计。系统采用模块化架构，支持Mp4和H.264格式的本地视频文件流式读取、帧级别分片、音视频分流传输，并通过QUIC协议实现到云端服务器的可靠传输。

核心设计原则：
- **流式处理**：避免大文件内存占用，支持任意大小的视频文件
- **极低延迟**：帧级别分片和QUIC协议确保最小传输延迟
- **高可靠性**：完善的错误处理和恢复机制
- **模块化架构**：组件间松耦合，便于维护和扩展

## Architecture

系统采用分层架构设计，包含以下主要层次：

```
┌─────────────────────────────────────────────────────────────┐
│                    User Interface Layer                     │
├─────────────────────────────────────────────────────────────┤
│                  Application Control Layer                  │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ Playback        │  │ Performance     │  │ Upload       │ │
│  │ Controller      │  │ Monitor         │  │ Manager      │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
├─────────────────────────────────────────────────────────────┤
│                   Stream Processing Layer                   │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ Video           │  │ Audio Video     │  │ Stream       │ │
│  │ Segmenter       │  │ Separator       │  │ Multiplexer  │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
├─────────────────────────────────────────────────────────────┤
│                    Transport Layer                          │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ QUIC            │  │ Connection      │  │ Retry        │ │
│  │ Transport       │  │ Manager         │  │ Handler      │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
├─────────────────────────────────────────────────────────────┤
│                      I/O Layer                              │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ File Stream     │  │ Network         │  │ Buffer       │ │
│  │ Reader          │  │ Interface       │  │ Manager      │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Protocol Specification

### QUIC传输协议规范

为确保与不同语言实现的服务端兼容，定义语言无关的传输协议规范：

#### 连接建立协议
```
1. 客户端发起QUIC连接到服务器端口
2. 使用TLS 1.3进行握手和认证
3. 协商应用层协议标识符：video-streaming/1.0
4. 设置连接参数：
   - max_concurrent_streams: 100
   - initial_max_data: 10MB
   - initial_max_stream_data: 1MB
   - idle_timeout: 30s
```

#### 流类型定义
```
Stream Type ID | Purpose        | Direction
0x00          | Control        | Bidirectional
0x01          | Video Data     | Unidirectional (Client->Server)
0x02          | Audio Data     | Unidirectional (Client->Server)
0x03          | Metadata       | Unidirectional (Client->Server)
0x04          | Status         | Unidirectional (Server->Client)
```

#### 消息格式规范

**控制消息格式 (Stream 0x00):**
```
+--+--+--+--+--+--+--+--+
|     Message Type      |
+--+--+--+--+--+--+--+--+
|     Message Length    |
+--+--+--+--+--+--+--+--+
|     Payload Data      |
+--+--+--+--+--+--+--+--+
```

**视频数据消息格式 (Stream 0x01):**
```
+--+--+--+--+--+--+--+--+
|     Segment ID (16B)  |
+--+--+--+--+--+--+--+--+
|     Timestamp (8B)    |
+--+--+--+--+--+--+--+--+
|     Duration (8B)     |
+--+--+--+--+--+--+--+--+
|     Frame Count (4B)  |
+--+--+--+--+--+--+--+--+
|     Flags (1B)        |
+--+--+--+--+--+--+--+--+
|     Data Length (4B)  |
+--+--+--+--+--+--+--+--+
|     Video Data        |
+--+--+--+--+--+--+--+--+
```

**音频数据消息格式 (Stream 0x02):**
```
+--+--+--+--+--+--+--+--+
|     Segment ID (16B)  |
+--+--+--+--+--+--+--+--+
|     Timestamp (8B)    |
+--+--+--+--+--+--+--+--+
|     Duration (8B)     |
+--+--+--+--+--+--+--+--+
|     Sample Rate (4B)  |
+--+--+--+--+--+--+--+--+
|     Channels (2B)     |
+--+--+--+--+--+--+--+--+
|     Data Length (4B)  |
+--+--+--+--+--+--+--+--+
|     Audio Data        |
+--+--+--+--+--+--+--+--+
```

#### 控制消息类型
```
Message Type | Name           | Description
0x01        | SESSION_START  | 开始传输会话
0x02        | SESSION_END    | 结束传输会话
0x03        | SEEK_REQUEST   | 请求跳转到指定位置
0x04        | RATE_CHANGE    | 改变播放速率
0x05        | PAUSE_REQUEST  | 暂停传输
0x06        | RESUME_REQUEST | 恢复传输
0x07        | ERROR_REPORT   | 错误报告
0x08        | STATS_REQUEST  | 请求统计信息
```

#### 状态响应格式
```
+--+--+--+--+--+--+--+--+
|     Status Code (2B)  |
+--+--+--+--+--+--+--+--+
|     Message Length    |
+--+--+--+--+--+--+--+--+
|     Status Message    |
+--+--+--+--+--+--+--+--+
```

#### 错误处理协议
```
Error Code | Name                    | Action
1001      | UNSUPPORTED_FORMAT      | 拒绝连接
1002      | INSUFFICIENT_BANDWIDTH  | 降低质量
1003      | STORAGE_FULL           | 暂停传输
1004      | AUTHENTICATION_FAILED   | 重新认证
1005      | SEGMENT_CORRUPTED      | 重传段
```

## Components and Interfaces

### File Stream Reader
负责视频文件的流式读取和格式验证。

**接口定义：**
```rust
use std::path::Path;
use tokio::fs::File;

pub trait FileStreamReader {
    async fn open_file(&self, file_path: &Path) -> Result<File, FileError>;
    async fn read_chunk(&self, handle: &mut File, size: usize) -> Result<Vec<u8>, FileError>;
    async fn get_file_info(&self, handle: &mut File) -> Result<VideoFileInfo, FileError>;
    async fn seek_to_position(&self, handle: &mut File, position: u64) -> Result<(), FileError>;
    async fn close_file(&self, handle: File) -> Result<(), FileError>;
}

#[derive(Debug, Clone)]
pub struct VideoFileInfo {
    pub duration: f64,
    pub resolution: Resolution,
    pub codec: String,
    pub frame_rate: f64,
    pub bit_rate: u64,
    pub has_audio: bool,
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}
```

### Video Segmenter
实现帧级别的视频分片处理。

**接口定义：**
```rust
use tokio_stream::Stream;
use uuid::Uuid;

pub trait VideoSegmenter {
    async fn segment_stream(
        &self,
        input: impl Stream<Item = Vec<u8>>,
        options: SegmentOptions,
    ) -> impl Stream<Item = VideoSegment>;
    
    fn find_key_frames(&self, buffer: &[u8]) -> Vec<usize>;
    fn create_segment(
        &self,
        buffer: Vec<u8>,
        start_frame: usize,
        end_frame: usize,
    ) -> Result<VideoSegment, SegmentError>;
}

#[derive(Debug, Clone)]
pub struct SegmentOptions {
    pub segment_mode: SegmentMode,
    pub target_size: Option<usize>,
    pub max_frames_per_segment: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum SegmentMode {
    Frame,
    Gop,
    Time,
}

#[derive(Debug, Clone)]
pub struct VideoSegment {
    pub id: Uuid,
    pub data: Vec<u8>,
    pub timestamp: f64,
    pub duration: f64,
    pub frame_count: usize,
    pub is_key_frame: bool,
    pub metadata: SegmentMetadata,
}
```

### Audio Video Separator
分离音频和视频流，创建独立的传输通道。

**接口定义：**
```rust
pub trait AudioVideoSeparator {
    async fn separate_streams(
        &self,
        input: impl Stream<Item = Vec<u8>>,
    ) -> Result<SeparatedStreams, SeparationError>;
    
    fn synchronize_streams(
        &self,
        video_segment: &VideoSegment,
        audio_segment: &AudioSegment,
    ) -> SyncInfo;
}

#[derive(Debug)]
pub struct SeparatedStreams {
    pub video_stream: Box<dyn Stream<Item = VideoSegment> + Unpin + Send>,
    pub audio_stream: Box<dyn Stream<Item = AudioSegment> + Unpin + Send>,
}

#[derive(Debug, Clone)]
pub struct AudioSegment {
    pub id: Uuid,
    pub data: Vec<u8>,
    pub timestamp: f64,
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub struct SyncInfo {
    pub video_timestamp: f64,
    pub audio_timestamp: f64,
    pub offset: f64,
}
```

### QUIC Transport
基于QUIC协议的网络传输层。

**接口定义：**
```rust
use std::net::SocketAddr;
use std::time::Duration;

pub trait QUICTransport {
    async fn connect(
        &self,
        server_address: SocketAddr,
        options: QUICOptions,
    ) -> Result<QUICConnection, TransportError>;
    
    async fn send_segment(
        &self,
        connection: &mut QUICConnection,
        segment: Segment,
    ) -> Result<(), TransportError>;
    
    async fn create_stream(
        &self,
        connection: &mut QUICConnection,
        stream_type: StreamType,
    ) -> Result<QUICStream, TransportError>;
    
    fn get_connection_stats(&self, connection: &QUICConnection) -> ConnectionStats;
}

#[derive(Debug, Clone)]
pub struct QUICOptions {
    pub max_concurrent_streams: u32,
    pub initial_max_data: u64,
    pub initial_max_stream_data: u64,
    pub idle_timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum StreamType {
    Video,
    Audio,
}

#[derive(Debug, Clone)]
pub enum Segment {
    Video(VideoSegment),
    Audio(AudioSegment),
}

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub rtt: Duration,
    pub bandwidth: u64,
    pub packet_loss: f64,
    pub congestion_window: u64,
}
```

### Playback Controller
处理SEEK和倍速控制功能。

**接口定义：**
```rust
pub trait PlaybackController {
    async fn seek(&mut self, position: f64) -> Result<(), PlaybackError>;
    async fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlaybackError>;
    fn get_drop_frame_strategy(&self, rate: f64) -> DropFrameStrategy;
    fn adjust_transmission_queue(
        &self,
        segments: Vec<VideoSegment>,
        playback_rate: f64,
    ) -> Vec<VideoSegment>;
}

#[derive(Debug, Clone)]
pub struct DropFrameStrategy {
    pub drop_b_frames: bool,
    pub drop_p_frames: bool,
    pub keep_key_frames_only: bool,
    pub adaptive_dropping: bool,
}
```

### Performance Monitor
监控和统计传输性能。

**接口定义：**
```rust
use std::time::Duration;

pub trait PerformanceMonitor {
    fn start_monitoring(&mut self);
    fn record_transmission(&mut self, segment: &Segment, latency: Duration);
    fn get_current_stats(&self) -> PerformanceStats;
    fn generate_report(&self) -> PerformanceReport;
    fn export_data(&self, format: ExportFormat) -> Result<String, ExportError>;
}

#[derive(Debug, Clone)]
pub struct PerformanceStats {
    pub current_upload_speed: f64,
    pub average_upload_speed: f64,
    pub current_latency: Duration,
    pub average_latency: Duration,
    pub total_bytes_transmitted: u64,
    pub segments_transmitted: u64,
}

#[derive(Debug, Clone)]
pub enum ExportFormat {
    Json,
    Csv,
}
```

## Data Models

### Core Data Structures

```rust
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

// 视频文件元数据
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub file_path: PathBuf,
    pub file_size: u64,
    pub duration: f64,
    pub resolution: Resolution,
    pub codec: VideoCodec,
    pub frame_rate: f64,
    pub bit_rate: u64,
    pub audio_tracks: Vec<AudioTrackInfo>,
    pub created_at: SystemTime,
    pub checksum: String,
}

// 分辨率信息
#[derive(Debug, Clone)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: String,
}

// 视频编码信息
#[derive(Debug, Clone)]
pub struct VideoCodec {
    pub name: String,
    pub profile: String,
    pub level: String,
    pub color_space: String,
    pub bit_depth: u8,
}

// 音频轨道信息
#[derive(Debug, Clone)]
pub struct AudioTrackInfo {
    pub index: u32,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_rate: u64,
    pub language: Option<String>,
}

// 传输会话
#[derive(Debug, Clone)]
pub struct TransmissionSession {
    pub id: Uuid,
    pub video_metadata: VideoMetadata,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub status: TransmissionStatus,
    pub progress: f64,
    pub current_position: f64,
    pub playback_rate: f64,
    pub total_segments: u64,
    pub transmitted_segments: u64,
    pub errors: Vec<TransmissionError>,
}

#[derive(Debug, Clone)]
pub enum TransmissionStatus {
    Preparing,
    Active,
    Paused,
    Completed,
    Failed,
}

// 传输错误
#[derive(Debug, Clone)]
pub struct TransmissionError {
    pub timestamp: SystemTime,
    pub error_type: ErrorType,
    pub message: String,
    pub segment_id: Option<Uuid>,
    pub retry_count: u32,
    pub resolved: bool,
}

#[derive(Debug, Clone)]
pub enum ErrorType {
    Network,
    Encoding,
    File,
    Server,
}
```

### Stream Processing Models

```rust
use std::collections::HashMap;
use tokio_stream::Stream;

// 流处理管道
#[derive(Debug)]
pub struct StreamPipeline {
    pub id: Uuid,
    pub stages: Vec<PipelineStage>,
    pub input_stream: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
    pub output_stream: Box<dyn Stream<Item = Vec<u8>> + Unpin + Send>,
    pub buffer_size: usize,
    pub backpressure_threshold: usize,
}

// 管道阶段
#[derive(Debug)]
pub struct PipelineStage {
    pub name: String,
    pub processor: Box<dyn StreamProcessor + Send + Sync>,
    pub input_buffer: Vec<Vec<u8>>,
    pub output_buffer: Vec<Vec<u8>>,
    pub metrics: StageMetrics,
}

pub trait StreamProcessor {
    fn process(&mut self, input: Vec<u8>) -> Result<Vec<u8>, ProcessingError>;
}

#[derive(Debug, Clone)]
pub struct StageMetrics {
    pub processed_bytes: u64,
    pub processing_time: Duration,
    pub error_count: u32,
}

// 缓冲区管理
#[derive(Debug)]
pub struct BufferManager {
    pub video_buffers: HashMap<Uuid, Vec<VideoSegment>>,
    pub audio_buffers: HashMap<Uuid, Vec<AudioSegment>>,
    pub max_buffer_size: usize,
    pub current_buffer_size: usize,
    pub buffer_health: BufferHealth,
}

#[derive(Debug, Clone)]
pub struct BufferHealth {
    pub video_buffer_level: f64,
    pub audio_buffer_level: f64,
    pub underrun_count: u32,
    pub overrun_count: u32,
    pub last_underrun: Option<SystemTime>,
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

基于需求分析，我识别出以下需要通过属性测试验证的正确性属性：

### File Processing Properties

**Property 1: Format validation consistency**
*For any* valid Mp4 or H264 video file, the file reader should correctly identify and validate the format without loading the entire file into memory
**Validates: Requirements 1.1, 1.2, 1.3**

**Property 2: Invalid format rejection**
*For any* unsupported video format file, the system should reject the file and provide appropriate error information
**Validates: Requirements 1.4**

**Property 3: Metadata extraction completeness**
*For any* valid video file, the extracted metadata should contain duration, resolution, and codec information
**Validates: Requirements 1.5**

**Property 4: Error detection reliability**
*For any* corrupted or unreadable video file, the system should detect the error and notify the user appropriately
**Validates: Requirements 1.6**

### Video Segmentation Properties

**Property 5: Frame-level segmentation**
*For any* video requiring low-latency transmission, the segmenter should produce frame-level segments
**Validates: Requirements 2.1**

**Property 6: Key frame alignment**
*For any* frame-level segmentation, each segment should start at a key frame position to ensure decode integrity
**Validates: Requirements 2.2**

**Property 7: Quality preservation**
*For any* video segmentation process, the original video quality and encoding parameters should be preserved
**Validates: Requirements 2.3**

**Property 8: Streaming segmentation**
*For any* video file, segmentation should begin producing segments before the entire file is loaded
**Validates: Requirements 2.4**

**Property 9: Metadata completeness**
*For any* completed segmentation, the generated index should contain frame-level metadata for all segments
**Validates: Requirements 2.5**

**Property 10: Adaptive segmentation**
*For any* network condition change, the segmenter should adjust granularity between frame-level and GOP-level appropriately
**Validates: Requirements 2.6**

**Property 11: Error recovery**
*For any* segmentation error, the system should record the error position and support resumption from that point
**Validates: Requirements 2.7**

### Audio-Video Separation Properties

**Property 12: Stream separation**
*For any* video file containing audio tracks, the separator should successfully extract independent audio and video streams
**Validates: Requirements 3.1**

**Property 13: Independent channel creation**
*For any* separated audio and video streams, the system should create independent transmission channels
**Validates: Requirements 3.2**

**Property 14: Audio transmission configuration**
*For any* audio stream transmission, the QUIC transport should use low-latency optimized configuration
**Validates: Requirements 3.3**

**Property 15: Video transmission configuration**
*For any* video stream transmission, the QUIC transport should use high-throughput optimized configuration
**Validates: Requirements 3.4**

**Property 16: Synchronization maintenance**
*For any* simultaneous audio-video transmission, timestamp synchronization information should be maintained
**Validates: Requirements 3.5**

### QUIC Transport Properties

**Property 17: QUIC connection establishment**
*For any* upload initiation, the transport should establish a valid QUIC connection with the cloud server
**Validates: Requirements 4.1**

**Property 18: Low-latency configuration**
*For any* established QUIC connection, the parameters should be configured to minimize latency
**Validates: Requirements 4.2**

**Property 19: Multiplexed transmission**
*For any* video segment transmission, the QUIC transport should utilize multiplexing for concurrent segment delivery
**Validates: Requirements 4.3**

**Property 20: Adaptive parameter adjustment**
*For any* network condition change, the QUIC transport should automatically adjust parameters to maintain optimal performance
**Validates: Requirements 4.4**

**Property 21: Connection recovery**
*For any* connection interruption, the QUIC transport should automatically reconnect and resume from the interruption point
**Validates: Requirements 4.5**

### Playback Control Properties

**Property 22: Seek accuracy**
*For any* seek request to a specific time position, the controller should locate the corresponding video segment and adjust the transmission queue accordingly
**Validates: Requirements 5.1**

**Property 23: Playback rate adjustment**
*For any* playback rate setting, the controller should adjust segment transmission rate to match the playback speed
**Validates: Requirements 5.2**

**Property 24: Frame dropping strategy**
*For any* playback rate exceeding network capacity, the controller should implement intelligent frame dropping
**Validates: Requirements 5.3**

**Property 25: Buffer management on seek**
*For any* seek operation, the controller should clear the current transmission buffer and requeue relevant segments
**Validates: Requirements 5.4**

**Property 26: Synchronization adjustment**
*For any* playback speed change, the controller should dynamically adjust audio-video synchronization strategy
**Validates: Requirements 5.5**

### Performance Monitoring Properties

**Property 27: Real-time speed calculation**
*For any* ongoing transmission, the performance monitor should accurately calculate and display current upload speed
**Validates: Requirements 6.1**

**Property 28: Latency measurement**
*For any* data packet transmission, the performance monitor should measure and record end-to-end latency
**Validates: Requirements 6.2**

**Property 29: Statistical report generation**
*For any* completed transmission, the performance monitor should generate a report containing average speed, peak speed, and latency distribution
**Validates: Requirements 6.3**

**Property 30: Anomaly detection**
*For any* network performance fluctuation, the performance monitor should detect anomalies and provide optimization recommendations
**Validates: Requirements 6.4**

**Property 31: Data export completeness**
*For any* user export request, the performance monitor should export complete and properly formatted performance data
**Validates: Requirements 6.5**

### Error Handling Properties

**Property 32: Network failure retry**
*For any* network connection failure, the system should implement retry mechanisms and record failure reasons
**Validates: Requirements 7.1**

**Property 33: Server error handling**
*For any* server error response, the system should parse error information and take appropriate recovery measures
**Validates: Requirements 7.2**

**Property 34: Storage monitoring**
*For any* insufficient local storage condition, the system should detect storage status and warn the user
**Validates: Requirements 7.3**

**Property 35: File integrity monitoring**
*For any* video file modification during transmission, the system should detect file changes and pause transmission
**Validates: Requirements 7.4**

**Property 36: Resource adaptation**
*For any* insufficient system resources condition, the system should adjust processing parameters to adapt to available resources
**Validates: Requirements 7.5**

## Error Handling

系统采用多层错误处理策略，确保与不同语言实现的服务端兼容：

### 错误分类
- **网络错误**：连接失败、超时、丢包
- **文件错误**：格式不支持、文件损坏、权限不足
- **编码错误**：分片失败、格式转换错误
- **服务器错误**：响应错误、认证失败、存储满
- **协议错误**：消息格式错误、版本不兼容

### 协议级错误处理
1. **版本协商**：客户端和服务端协商支持的协议版本
2. **消息验证**：所有消息都包含校验和以确保完整性
3. **超时处理**：定义明确的超时时间和重试策略
4. **状态同步**：定期同步客户端和服务端状态

### 错误处理机制
1. **重试策略**：指数退避算法，最大重试次数限制
2. **降级处理**：网络条件差时自动降低质量或分片粒度
3. **断点续传**：记录传输进度，支持从中断点恢复
4. **用户通知**：清晰的错误信息和建议操作
5. **协议回退**：不支持新协议时回退到兼容版本

### 跨语言兼容性保证
- 使用标准的网络字节序（大端序）
- 所有时间戳使用Unix时间戳（毫秒精度）
- 字符串使用UTF-8编码
- 浮点数使用IEEE 754标准
- 消息长度字段使用固定字节数

### 错误恢复流程
```
错误检测 → 错误分类 → 协议验证 → 选择恢复策略 → 执行恢复 → 验证恢复 → 继续处理
```

## Testing Strategy

系统采用双重测试方法，结合单元测试和基于属性的测试：

### 单元测试方法
- **具体示例验证**：测试特定输入的预期输出
- **边界条件测试**：测试极限情况和边界值
- **集成点测试**：验证组件间接口的正确性
- **错误条件测试**：验证错误处理的正确性

### 基于属性的测试方法
- **使用proptest库**：Rust的属性测试框架
- **最小100次迭代**：每个属性测试运行至少100次随机输入
- **智能生成器**：为视频文件、网络条件等创建专门的测试数据生成器
- **属性标记**：每个属性测试都标记对应的设计文档属性编号

### 测试配置要求
- 每个属性测试必须运行最少100次迭代以确保随机性覆盖
- 使用注释格式标记属性测试：`**Feature: video-streaming-uploader, Property {number}: {property_text}**`
- 每个正确性属性必须对应一个独立的属性测试
- 属性测试应尽量避免使用模拟，测试真实功能
- 使用`proptest`宏和策略生成器创建复杂的测试数据

### 测试数据生成策略
- **视频文件生成器**：创建各种格式、大小、质量的测试视频
- **网络条件模拟器**：模拟不同的带宽、延迟、丢包率
- **错误注入器**：在关键点注入各种类型的错误
- **性能基准生成器**：生成性能测试的基准数据

### 测试覆盖目标
- 所有36个正确性属性都有对应的属性测试
- 关键路径的单元测试覆盖率达到90%以上
- 错误处理路径的测试覆盖率达到80%以上
- 性能测试覆盖所有关键性能指标