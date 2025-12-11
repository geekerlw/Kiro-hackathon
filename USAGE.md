# 视频流上传系统使用说明

## 概述

这个系统实现了按需视频流上传功能，包含两个主要组件：

1. **模拟平台服务器** (`mock-server`) - 模拟接收端平台，主动请求客户端上传文件
2. **按需上传客户端** (`client`) - 等待平台请求，然后开始上传指定文件

## 系统架构

```
┌─────────────────┐    QUIC协议    ┌─────────────────┐
│   平台服务器     │ ◄──────────── │   上传客户端     │
│  (mock-server)  │               │    (client)     │
│                 │               │                 │
│ • 文件请求       │               │ • 文件注册       │
│ • 播放控制       │               │ • 按需上传       │
│ • 统计监控       │               │ • 流式传输       │
└─────────────────┘               └─────────────────┘
```

## 编译项目

```bash
# 编译所有组件
cargo build --release

# 或者编译特定组件
cargo build --bin mock-server --release
cargo build --bin client --release
```

## 使用方法

### 1. 启动模拟平台服务器

```bash
# 使用默认配置启动 (127.0.0.1:8080)
cargo run --bin mock-server

# 指定端口和地址
cargo run --bin mock-server -- --host 0.0.0.0 --port 9090
```

服务器启动后会显示交互式控制台，支持以下命令：

- `list` - 列出所有活跃会话
- `query [session_id] [filter]` - 查询客户端可用文件列表
- `request <file_path> [session_id]` - 请求客户端上传指定文件
- `seek <position> [session_id]` - 发送跳转命令（秒）
- `rate <rate> [session_id]` - 改变播放速率（倍速）
- `pause [session_id]` - 暂停上传
- `resume [session_id]` - 恢复上传
- `stop [session_id]` - 停止上传
- `stats [session_id]` - 显示会话统计信息
- `quit` - 退出服务器

**注意**: 如果没有提供session_id，系统会自动使用第一个可用的会话。

### 2. 启动上传客户端

```bash
# 连接到默认服务器 (127.0.0.1:8080)
cargo run --bin client

# 连接到指定服务器
cargo run --bin client -- --server 192.168.1.100:9090

# 注册指定的视频文件
cargo run --bin client -- --files "video1.mp4,video2.h264,stream.mp4"

# 扫描目录中的所有视频文件
cargo run --bin client -- --directory "/path/to/videos"

# 组合使用
cargo run --bin client -- --server 192.168.1.100:9090 --directory "/home/user/videos"
```

客户端启动后会显示交互式控制台，支持以下命令：

- `status` - 显示客户端状态
- `sessions` - 列出活跃的上传会话
- `files` - 列出已注册的文件
- `register <file_path>` - 注册新的视频文件
- `quit` - 退出客户端

## 完整使用流程示例

### 步骤1：启动服务器
```bash
# 终端1
cargo run --bin mock-server
```

输出：
```
INFO mock_platform_server: Mock platform server started on 127.0.0.1:8080
INFO mock_platform_server: Registered test file: sample_video_1.mp4
INFO mock_platform_server: Registered test file: sample_video_2.mp4
INFO mock_platform_server: Registered test file: test_stream.h264
Mock server is running. Available commands:
  request <session_id> <file_path> - Request file upload from client
  ...
```

### 步骤2：启动客户端
```bash
# 终端2
cargo run --bin client -- --files "sample_video_1.mp4,sample_video_2.mp4"
```

输出：
```
INFO client: Starting video streaming client...
INFO client: Connecting to platform server at: 127.0.0.1:8080
INFO client: Successfully connected to platform server
INFO client: Registered file: "sample_video_1.mp4"
INFO client: Registered file: "sample_video_2.mp4"
Available files for streaming:
  - sample_video_1.mp4
  - sample_video_2.mp4
Client is ready and waiting for platform requests.
```

### 步骤3：查看连接的会话
在服务器控制台输入：
```
list
```

输出：
```
INFO mock_platform_server: Active sessions:
INFO mock_platform_server:   550e8400-e29b-41d4-a716-446655440000 - Connected
```

### 步骤4：查询客户端可用文件
在服务器控制台输入（可以使用session_id或直接查询）：
```
query
```

输出：
```
INFO mock_platform_server: Querying file list from session 550e8400-e29b-41d4-a716-446655440000
INFO mock_platform_server: Sending file list response with 2 files
INFO mock_platform_server:   - sample_video_1.mp4 (52428800 bytes, format: mp4)
INFO mock_platform_server:   - sample_video_2.mp4 (104857600 bytes, format: mp4)
```

### 步骤5：服务器请求文件上传
在服务器控制台输入（简化命令）：
```
request sample_video_1.mp4
```

系统会自动进行文件分片和传输：
```
INFO client: Starting file segmentation and transmission for: sample_video_1.mp4
INFO client: File sample_video_1.mp4 will be split into 50 segments
INFO client: Starting transmission of 50 segments
INFO client: Transmitted video segment 1/50
INFO client: Transmitted video segment 2/50
INFO client: Transmitted audio segment 3/50
...
```

### 步骤6：控制播放
```bash
# 跳转到30秒位置（会影响当前传输位置）
seek 30.0

# 设置2倍速播放（会加快传输速度）
rate 2.0

# 暂停上传（停止分片传输）
pause

# 恢复上传（继续分片传输）
resume
```

### 步骤7：查看统计信息
```bash
# 查看所有会话统计
stats
```

## 支持的视频格式

- **MP4** (.mp4) - 标准MP4容器格式
- **H.264** (.h264) - 原始H.264视频流
- **AVI** (.avi) - AVI容器格式
- **MOV** (.mov) - QuickTime格式
- **MKV** (.mkv) - Matroska格式

## 网络协议

系统使用QUIC协议进行传输，具有以下特性：

- **低延迟** - 0-RTT连接建立
- **多路复用** - 单连接多流传输
- **可靠传输** - 自动重传和拥塞控制
- **加密安全** - 内置TLS 1.3加密

### 协议消息格式

#### 控制消息
```
+--+--+--+--+--+--+--+--+
|     Message Type      |
+--+--+--+--+--+--+--+--+
|     Message Length    |
+--+--+--+--+--+--+--+--+
|     Payload Data      |
+--+--+--+--+--+--+--+--+
```

#### 视频数据
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
|     Video Data        |
+--+--+--+--+--+--+--+--+
```

## 性能监控

系统提供实时性能监控功能：

- **上传速度** - 实时和平均上传速度
- **网络延迟** - 端到端传输延迟
- **分片统计** - 视频/音频分片数量
- **错误统计** - 传输错误和重试次数

## 故障排除

### 常见问题

1. **连接失败**
   ```
   Error: Failed to connect to server
   ```
   - 检查服务器是否启动
   - 确认网络地址和端口正确
   - 检查防火墙设置

2. **文件注册失败**
   ```
   Error: Failed to register file: UnsupportedFormat
   ```
   - 确认文件格式受支持
   - 检查文件是否存在且可读
   - 验证文件不是损坏的

3. **上传中断**
   ```
   Error: Upload session failed
   ```
   - 检查网络连接稳定性
   - 确认文件在上传过程中未被修改
   - 查看详细错误日志

### 调试模式

启用详细日志输出：
```bash
RUST_LOG=debug cargo run --bin client
RUST_LOG=debug cargo run --bin mock-server
```

## 配置选项

### 服务器配置
- `--host` - 监听地址 (默认: 127.0.0.1)
- `--port` - 监听端口 (默认: 8080)

### 客户端配置
- `--server` - 服务器地址 (默认: 127.0.0.1:8080)
- `--files` - 要注册的文件列表 (逗号分隔)
- `--directory` - 要扫描的视频文件目录

## 扩展开发

### 添加新的视频格式支持
1. 在 `file_reader.rs` 中添加格式检测逻辑
2. 在 `segmenter.rs` 中实现对应的分片算法
3. 更新 `types.rs` 中的格式枚举

### 自定义传输协议
1. 实现 `QUICTransport` trait
2. 定义新的消息类型在 `types.rs`
3. 更新协议处理逻辑

### 性能优化
- 调整QUIC连接参数
- 优化分片大小和缓冲区
- 实现自适应码率控制