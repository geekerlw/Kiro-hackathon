# 视频流上传系统使用说明

## 概述

这个系统实现了按需视频流上传功能，支持精确seek定位和无缝上传续传，包含两个主要组件：

1. **模拟平台服务器** (`mock-server`) - 模拟接收端平台，主动请求客户端上传文件
2. **按需上传客户端** (`client`) - 等待平台请求，然后开始上传指定文件

## 🆕 新功能特性

### 增强Seek功能
- **精确关键帧定位** - 基于关键帧索引的亚秒级精确seek
- **自动关键帧对齐** - 非关键帧位置自动对齐到最近关键帧
- **多种优化策略** - Full、Sparse、Adaptive、Hierarchical索引优化
- **详细Seek结果** - 包含请求位置、实际位置、精度和执行时间

### 完整Seek-Continue工作流程
- **查询文件列表** - 服务器查询客户端可用文件
- **请求文件上传** - 服务器请求特定文件上传
- **精确Seek操作** - 跳转到指定时间位置
- **自动续传** - 从新位置自动继续上传

### 性能提升
- **高吞吐量传输** - 实现1+ Gbps传输速率
- **实时统计监控** - 详细的传输性能统计
- **智能分片策略** - 根据文件格式优化分片方式

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

#### 基础命令
- `list` - 列出所有活跃会话
- `query [session_id] [filter]` - 查询客户端可用文件列表
- `request <file_path> [session_id]` - 请求客户端上传指定文件
- `stats [session_id]` - 显示会话统计信息
- `quit` - 退出服务器

#### 🆕 增强播放控制命令
- `seek <position> [session_id]` - **精确跳转**到指定时间位置（秒）
  - 支持亚秒级精度（如：`seek 30.5`）
  - 自动使用关键帧索引进行精确定位
  - 跳转后自动从新位置继续上传
- `rate <rate> [session_id]` - 改变播放速率（倍速）
  - 支持0.1x到10x倍速（如：`rate 2.0`、`rate 0.5`）
  - 与seek操作无缝集成
- `pause [session_id]` - 暂停上传
- `resume [session_id]` - 恢复上传
- `stop [session_id]` - 停止上传

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

## 🆕 增强Seek功能演示

### 运行完整工作流程演示
```bash
# 运行完整的seek-and-continue演示
cargo run --example seek_and_continue_demo
```

这个演示展示了完整的工作流程：
1. 查询文件列表
2. 请求文件上传（自动构建关键帧索引）
3. 执行多个精确seek操作
4. 从新位置自动继续上传
5. 测试不同播放速率与seek的结合

### 运行其他演示
```bash
# 增强seek功能演示（客户端-服务器模式）
cargo run --example enhanced_seek_client_server_demo

# 关键帧索引演示
cargo run --example keyframe_index_demo

# 基础seek功能演示
cargo run --example enhanced_seek_demo
```

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

### 步骤6：🆕 增强播放控制和精确Seek
```bash
# 精确跳转到30.5秒位置（使用关键帧索引）
seek 30.5

# 系统输出示例：
# INFO: Precise seek completed: requested=30.500s, actual=30.467s, precision=0.999
# INFO: Restarting upload from new position: 30.467s
# INFO: Keyframe index built successfully: 18 keyframes, precision: 0.031s

# 设置2倍速播放并跳转
rate 2.0
seek 15.0

# 测试不同倍速与seek的结合
rate 0.5    # 0.5倍速
seek 45.2   # 跳转到45.2秒

rate 4.0    # 4倍速
seek 20.0   # 跳转到20秒

# 暂停、跳转、恢复的组合操作
pause       # 暂停上传
seek 10.5   # 跳转到10.5秒
resume      # 从新位置恢复上传
```

#### 🆕 Seek操作特性
- **亚秒级精度** - 支持小数点精度（如30.5秒）
- **自动关键帧对齐** - 自动对齐到最近的关键帧确保解码完整性
- **精度反馈** - 显示请求位置vs实际位置的精度
- **自动续传** - 跳转后自动从新位置继续上传
- **性能统计** - 显示seek执行时间和传输统计

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

## 🆕 增强性能监控

系统提供实时性能监控功能：

### 基础性能指标
- **上传速度** - 实时和平均上传速度
- **网络延迟** - 端到端传输延迟
- **分片统计** - 视频/音频分片数量
- **错误统计** - 传输错误和重试次数

### 🆕 增强Seek性能指标
- **关键帧索引统计** - 关键帧数量、索引精度、内存使用
- **Seek操作性能** - 执行时间、精度达成率
- **传输续传统计** - 从新位置续传的性能数据
- **高吞吐量监控** - 实时显示1+ Gbps传输速率

### 性能示例输出
```
=== 传输性能统计 ===
关键帧索引: 18个关键帧, 精度: 0.031s
总分片数: 33个分片
传输大小: 8,502,716 bytes
传输时间: 0.05s
平均分片时间: 1.55ms/分片
整体吞吐量: 1,333.8 Mbps
峰值吞吐量: 2,097.2 Mbps
Seek精度: 99.9%
```

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

### 🆕 Seek相关问题

4. **Seek位置超出范围**
   ```
   Error: Invalid seek position: 35
   ```
   - 检查seek位置是否超出视频总时长
   - 使用`stats`命令查看文件总时长
   - 确保seek位置为正数

5. **关键帧索引构建失败**
   ```
   Warning: Failed to build keyframe index. Falling back to basic seek.
   ```
   - 系统会自动回退到基础seek功能
   - 检查视频文件格式是否支持关键帧索引
   - 对于不支持的格式，仍可使用基础seek

6. **Seek精度较低**
   ```
   INFO: Seek precision achieved: 0.85
   ```
   - 检查视频文件的关键帧密度
   - 考虑使用Full优化策略提高精度
   - 验证关键帧索引是否成功构建

### 调试模式

启用详细日志输出：
```bash
RUST_LOG=debug cargo run --bin client
RUST_LOG=debug cargo run --bin mock-server

# 🆕 启用seek相关的详细调试信息
RUST_LOG=debug cargo run --example seek_and_continue_demo
RUST_LOG=debug cargo run --example enhanced_seek_client_server_demo
```

### 🆕 测试套件

运行完整的测试套件验证功能：
```bash
# 运行所有测试（包括seek相关测试）
cargo test --lib

# 运行特定的seek测试
cargo test seek --lib
cargo test controller --lib
cargo test file_reader --lib
cargo test seek_integration --lib

# 运行性能基准测试
cargo test --release seek_performance
```

当前测试覆盖：**79个测试全部通过** ✅

## 配置选项

### 服务器配置
- `--host` - 监听地址 (默认: 127.0.0.1)
- `--port` - 监听端口 (默认: 8080)

### 客户端配置
- `--server` - 服务器地址 (默认: 127.0.0.1:8080)
- `--files` - 要注册的文件列表 (逗号分隔)
- `--directory` - 要扫描的视频文件目录

## 🆕 API参考

### PlaybackController方法

#### 增强Seek方法
```rust
// 精确关键帧seek
async fn seek_to_keyframe(position: f64, index: &KeyframeIndex) -> Result<SeekResult, PlaybackError>

// 自动关键帧对齐seek
async fn seek_with_alignment(position: f64, index: &KeyframeIndex) -> Result<SeekResult, PlaybackError>

// 查找最近关键帧
fn find_nearest_keyframe(timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry>

// 优化策略关键帧查找
fn find_optimal_keyframe(timestamp: f64, index: &KeyframeIndex) -> Option<KeyframeEntry>
```

#### SeekResult结构
```rust
pub struct SeekResult {
    pub requested_time: f64,        // 请求的seek位置
    pub actual_time: f64,           // 实际seek后的位置
    pub keyframe_offset: u64,       // 使用的关键帧文件偏移
    pub precision_achieved: f64,    // 达成的精度 (0.0-1.0)
    pub keyframe_used: KeyframeEntry, // 使用的关键帧信息
    pub execution_time: Duration,   // seek操作执行时间
}
```

### 关键帧索引优化策略
- **Full** - 完整关键帧索引，最高精度
- **Sparse** - 稀疏索引，内存优化
- **Adaptive** - 自适应策略，基于可用内存
- **Hierarchical** - 分层索引，适合大文件

## 扩展开发

### 🆕 增强Seek功能扩展
1. 在 `controller.rs` 中添加新的seek策略
2. 在 `file_reader.rs` 中实现关键帧索引优化
3. 更新 `types.rs` 中的SeekResult和KeyframeIndex结构

### 添加新的视频格式支持
1. 在 `file_reader.rs` 中添加格式检测逻辑
2. 在 `segmenter.rs` 中实现对应的分片算法
3. 更新 `types.rs` 中的格式枚举
4. 🆕 为新格式实现关键帧索引支持

### 自定义传输协议
1. 实现 `QUICTransport` trait
2. 定义新的消息类型在 `types.rs`
3. 更新协议处理逻辑
4. 🆕 集成seek-continue工作流程支持

### 性能优化
- 调整QUIC连接参数
- 优化分片大小和缓冲区
- 实现自适应码率控制
- 🆕 优化关键帧索引内存使用
- 🆕 实现预测性关键帧缓存