# 视频流上传系统平台集成协议规范

## 概述

本文档描述了视频流上传系统的完整协议规范，供平台端开发者实现与客户端的交互。系统基于QUIC协议实现低延迟、高可靠性的视频流传输。

## 系统架构

### 核心组件
- **客户端 (Client)**: 视频文件持有方，响应平台请求进行按需上传
- **平台服务器 (Platform Server)**: 主动请求视频文件，接收并处理上传的视频流
- **QUIC传输层**: 提供加密、多路复用、低延迟的网络传输

### 交互模式
- **按需上传**: 平台主动请求，客户端响应上传
- **实时控制**: 支持播放控制（SEEK、倍速、暂停/恢复）
- **分片传输**: 帧级分片，33ms延迟，适合实时流媒体

## 网络协议

### QUIC配置
```
协议: QUIC over UDP
加密: TLS 1.3
ALPN: "video-streaming/1.0"
端口: 8080 (默认)
```

### 连接参数
```rust
QUICOptions {
    max_concurrent_streams: 100,
    initial_max_data: 50 * 1024 * 1024,      // 50MB - 增加以支持大文件传输
    initial_max_stream_data: 5 * 1024 * 1024, // 5MB - 增加以支持大分片传输
    idle_timeout: 30秒,
}
```

## 协议消息格式

### 基础消息结构
所有协议消息均采用JSON序列化，通过QUIC单向流传输：

```json
{
  "message_type": "消息类型枚举值",
  "payload": "Base64编码的载荷数据",
  "sequence_number": "消息序列号(u64)",
  "timestamp": "ISO8601时间戳",
  "session_id": "会话UUID"
}
```

### 消息类型定义

#### 1. 会话管理消息

**SessionStart (0x01) - 会话开始**
```json
{
  "message_type": "SessionStart",
  "payload": "Q0xJRU5UX1JFQURZ",  // "CLIENT_READY"
  "sequence_number": 1,
  "timestamp": "2025-12-11T01:16:25Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**SessionEnd (0x02) - 会话结束**
```json
{
  "message_type": "SessionEnd",
  "payload": "",
  "sequence_number": 999,
  "timestamp": "2025-12-11T01:20:30Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

#### 2. 文件管理消息

**FileListQuery (0x0D) - 查询文件列表**
```json
{
  "message_type": "FileListQuery",
  "payload": "eyJmaWx0ZXIiOiAibXA0In0=",  // {"filter": "mp4"}
  "sequence_number": 2,
  "timestamp": "2025-12-11T01:16:30Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

载荷格式：
```json
{
  "filter": "可选的文件过滤器字符串"
}
```

**FileListResponse (0x0E) - 文件列表响应**
```json
{
  "message_type": "FileListResponse",
  "payload": "Base64编码的文件列表",
  "sequence_number": 3,
  "timestamp": "2025-12-11T01:16:31Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

载荷格式：
```json
{
  "files": [
    {
      "file_path": "test_videos/video1.mp4",
      "file_size": 1048576,
      "duration": 60.5,
      "format": "mp4",
      "available": true
    }
  ]
}
```

**FileRequest (0x0B) - 请求文件上传**
```json
{
  "message_type": "FileRequest",
  "payload": "Base64编码的文件请求",
  "sequence_number": 4,
  "timestamp": "2025-12-11T01:16:35Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

载荷格式：
```json
{
  "file_path": "test_videos/video1.mp4",
  "priority": 1,
  "seek_position": 30.0,  // 可选，从30秒开始
  "playback_rate": 1.0    // 播放倍速
}
```

#### 3. 播放控制消息

**PlaybackControl (0x0C) - 播放控制**
```json
{
  "message_type": "PlaybackControl",
  "payload": "Base64编码的控制命令",
  "sequence_number": 5,
  "timestamp": "2025-12-11T01:17:00Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

载荷格式（支持多种控制命令）：
```json
// SEEK命令
{
  "Seek": {
    "position": 45.5
  }
}

// 倍速控制
{
  "SetRate": {
    "rate": 2.0
  }
}

// 暂停
"Pause"

// 恢复
"Resume"

// 停止
"Stop"
```

## 视频分片传输协议

### 分片数据格式

客户端将视频文件分片后通过QUIC流传输，每个分片包含完整的协议头：

#### 视频分片格式 (总计42字节头部)
```
[1字节]  流类型标识符: 0x01 (视频)
[16字节] 分片ID: UUID字节数组
[8字节]  时间戳: f64大端序 (秒)
[8字节]  持续时间: f64大端序 (秒)
[4字节]  帧数量: u32大端序
[1字节]  标志位: 0x01=关键帧, 0x00=普通帧
[4字节]  数据长度: u32大端序
[N字节]  实际视频数据
```

#### 音频分片格式 (总计43字节头部)
```
[1字节]  流类型标识符: 0x02 (音频)
[16字节] 分片ID: UUID字节数组
[8字节]  时间戳: f64大端序 (秒)
[8字节]  持续时间: f64大端序 (秒)
[4字节]  采样率: u32大端序 (Hz)
[2字节]  声道数: u16大端序
[4字节]  数据长度: u32大端序
[N字节]  实际音频数据
```

### 分片策略
- **分片大小**: 8KB/片 (视频), 512B/片 (音频)
- **分片间隔**: 33ms (30fps)
- **关键帧**: 每30个分片一个关键帧 (每秒)
- **延迟优化**: 帧级分片，适合实时流媒体

## 平台端实现指南

### 1. 服务器启动

```rust
// 创建服务器配置
let server_config = create_server_config()?;
let endpoint = Endpoint::server(server_config, "127.0.0.1:8080".parse()?)?;

// 监听连接
while let Some(conn) = endpoint.accept().await {
    tokio::spawn(handle_client_connection(conn));
}
```

### 2. 连接处理

```rust
async fn handle_client_connection(connecting: quinn::Connecting) -> Result<(), Error> {
    let connection = connecting.await?;
    let session_id = Uuid::new_v4();
    
    // 处理协议消息和分片数据
    loop {
        match connection.accept_uni().await {
            Ok(recv_stream) => {
                tokio::spawn(handle_stream(session_id, recv_stream));
            }
            Err(_) => break,
        }
    }
}
```

### 3. 消息解析

```rust
async fn handle_stream(session_id: Uuid, mut stream: RecvStream) -> Result<(), Error> {
    let data = stream.read_to_end(1024 * 1024).await?;
    
    // 尝试解析协议消息
    if let Ok(protocol_msg) = serde_json::from_slice::<ProtocolMessage>(&data) {
        match protocol_msg.message_type {
            MessageType::SessionStart => handle_session_start(session_id).await?,
            MessageType::FileListResponse => handle_file_list(protocol_msg.payload).await?,
            _ => {}
        }
        return Ok(());
    }
    
    // 解析分片数据
    if data.len() > 8 {
        let stream_type = match data[0] {
            0x01 => StreamType::Video,
            0x02 => StreamType::Audio,
            _ => return Err("Unknown stream type".into()),
        };
        
        let segment = parse_segment_data(&data[1..], stream_type)?;
        write_segment_to_file(&segment).await?;
    }
}
```

### 4. 分片数据解析

```rust
fn parse_segment_data(data: &[u8], stream_type: StreamType) -> Result<SegmentData, Error> {
    match stream_type {
        StreamType::Video => {
            // 解析视频分片 (42字节头部)
            if data.len() < 41 { return Err("Data too short".into()); }
            
            let id = Uuid::from_bytes(data[0..16].try_into()?);
            let timestamp = f64::from_be_bytes(data[16..24].try_into()?);
            let duration = f64::from_be_bytes(data[24..32].try_into()?);
            let data_length = u32::from_be_bytes(data[37..41].try_into()?) as usize;
            let video_data = data[41..41+data_length].to_vec();
            
            Ok(SegmentData { id, timestamp, duration, data: video_data })
        }
        StreamType::Audio => {
            // 解析音频分片 (43字节头部)
            if data.len() < 42 { return Err("Data too short".into()); }
            
            let id = Uuid::from_bytes(data[0..16].try_into()?);
            let timestamp = f64::from_be_bytes(data[16..24].try_into()?);
            let duration = f64::from_be_bytes(data[24..32].try_into()?);
            let data_length = u32::from_be_bytes(data[38..42].try_into()?) as usize;
            let audio_data = data[42..42+data_length].to_vec();
            
            Ok(SegmentData { id, timestamp, duration, data: audio_data })
        }
    }
}
```

### 5. 文件重组

```rust
async fn write_segment_to_file(segment: &SegmentData) -> Result<(), Error> {
    let output_path = format!("recv_videos/{}_video.mp4", session_id);
    
    // 只写入纯净的视频数据，不包含协议头
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)
        .await?;
        
    file.write_all(&segment.data).await?;
    file.flush().await?;
}
```

## 交互流程示例

### 完整的文件上传流程

```
1. 客户端连接服务器
   Client -> Server: QUIC连接建立

2. 会话握手
   Client -> Server: SessionStart消息

3. 查询可用文件
   Server -> Client: FileListQuery消息
   Client -> Server: FileListResponse消息

4. 请求文件上传
   Server -> Client: FileRequest消息

5. 分片传输
   Client -> Server: 视频分片流 (连续)
   Client -> Server: 音频分片流 (连续)

6. 播放控制 (可选)
   Server -> Client: PlaybackControl消息

7. 传输完成
   Server: 生成接收报告
   Client/Server: SessionEnd消息
```

## 错误处理

### 错误码定义
- `0x01`: 协议版本不匹配
- `0x02`: 文件不存在
- `0x03`: 权限不足
- `0x04`: 网络错误
- `0x05`: 数据格式错误

### 错误响应格式
```json
{
  "message_type": "ErrorReport",
  "payload": "Base64编码的错误信息",
  "sequence_number": 100,
  "timestamp": "2025-12-11T01:18:00Z",
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

## 性能优化建议

### 网络优化
- 使用QUIC的多路复用特性并行传输
- 配置适当的拥塞控制算法
- 启用0-RTT连接恢复

### 存储优化
- 使用异步I/O写入文件
- 实现分片缓冲区管理
- 支持断点续传

### 监控指标
- 传输速率 (KB/s)
- 分片丢失率
- 端到端延迟
- 连接稳定性

## 安全考虑

### 传输安全
- TLS 1.3加密所有数据
- 证书验证 (生产环境)
- 会话密钥轮换

### 访问控制
- 会话ID验证
- 文件访问权限检查
- 速率限制

## 兼容性说明

### 协议版本
- 当前版本: 1.0.0
- 向后兼容策略
- 版本协商机制

### 平台支持
- Linux/macOS/Windows
- Rust/C++/Python/Go客户端库
- Docker容器化部署

---

本协议规范为视频流上传系统的完整技术实现提供了详细指导，确保平台端能够正确实现与客户端的交互。