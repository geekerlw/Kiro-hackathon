# H.264编码器修复需求文档

## 简介

当前的纯Rust H.264编码器生成的视频文件无法被VLC等播放器正确播放，需要修复SPS/PPS/帧数据的格式问题。

## 术语表

- **SPS**: Sequence Parameter Set，序列参数集
- **PPS**: Picture Parameter Set，图像参数集  
- **NAL**: Network Abstraction Layer，网络抽象层
- **UE编码**: Unsigned Exponential Golomb编码
- **SE编码**: Signed Exponential Golomb编码
- **IDR**: Instantaneous Decoder Refresh，即时解码器刷新帧
- **切片**: H.264中的基本编码单元

## 需求

### 需求1: 修复SPS生成

**用户故事**: 作为视频播放器，我需要正确的SPS数据，以便能够解析视频流的基本参数。

#### 验收标准

1. WHEN 生成SPS数据 THEN 系统SHALL使用正确的H.264 Baseline Profile格式
2. WHEN 编码图像尺寸 THEN 系统SHALL使用正确的UE编码格式
3. WHEN 设置SPS ID THEN 系统SHALL确保ID在0-31范围内
4. WHEN 包含必要字段 THEN 系统SHALL包含所有Baseline Profile要求的SPS字段
5. WHEN 生成SPS THEN 系统SHALL确保数据能被标准H.264解码器解析

### 需求2: 修复PPS生成

**用户故事**: 作为视频解码器，我需要正确的PPS数据，以便能够解码图像参数。

#### 验收标准

1. WHEN 生成PPS数据 THEN 系统SHALL使用正确的H.264格式
2. WHEN 引用SPS THEN 系统SHALL使用正确的SPS ID引用
3. WHEN 设置PPS ID THEN 系统SHALL确保ID在0-255范围内
4. WHEN 编码量化参数 THEN 系统SHALL使用正确的SE编码
5. WHEN 生成PPS THEN 系统SHALL确保与SPS兼容

### 需求3: 修复UE/SE编码

**用户故事**: 作为H.264解析器，我需要正确的Exponential Golomb编码，以便能够解析参数值。

#### 验收标准

1. WHEN 进行UE编码 THEN 系统SHALL使用标准的Exponential Golomb算法
2. WHEN 进行SE编码 THEN 系统SHALL正确处理有符号数值
3. WHEN 编码值为0 THEN 系统SHALL输出单个比特1
4. WHEN 编码非零值 THEN 系统SHALL使用正确的前导零和数据位
5. WHEN 输出编码结果 THEN 系统SHALL确保字节对齐正确

### 需求4: 修复帧数据格式

**用户故事**: 作为视频播放器，我需要正确的帧数据格式，以便能够显示视频内容。

#### 验收标准

1. WHEN 生成IDR帧 THEN 系统SHALL包含正确的切片头
2. WHEN 生成P帧 THEN 系统SHALL使用正确的帧间预测格式
3. WHEN 编码宏块数据 THEN 系统SHALL使用标准的H.264宏块结构
4. WHEN 生成切片 THEN 系统SHALL确保切片类型在有效范围内
5. WHEN 输出帧数据 THEN 系统SHALL确保数据能被标准解码器处理

### 需求5: 验证播放兼容性

**用户故事**: 作为用户，我需要生成的H.264文件能被主流播放器播放，以便验证直播流功能。

#### 验收标准

1. WHEN 生成H.264文件 THEN VLC播放器SHALL能够正确播放
2. WHEN 使用ffprobe检查 THEN 系统SHALL报告正确的视频参数
3. WHEN 播放视频 THEN 系统SHALL显示屏幕录制内容和时间戳叠加
4. WHEN 转换为MP4 THEN ffmpeg SHALL能够无错误转换
5. WHEN 分析文件 THEN 系统SHALL包含有效的SPS/PPS/IDR/P帧结构