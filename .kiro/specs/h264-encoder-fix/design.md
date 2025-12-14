# H.264编码器修复设计文档

## 概述

本设计文档描述了如何修复当前纯Rust H.264编码器的格式问题，使生成的视频文件能够被VLC等主流播放器正确播放。

## 架构

### 当前问题分析

1. **SPS格式错误**: UE编码实现不正确，缺少必要字段
2. **PPS格式错误**: 结构不完整，字段值不正确  
3. **帧数据错误**: 缺少切片头，宏块格式不标准
4. **编码算法错误**: UE/SE编码实现有缺陷

### 修复策略

采用**标准兼容**的方法，确保生成的H.264流完全符合ITU-T H.264标准。

## 组件和接口

### 1. Exponential Golomb编码器

```rust
pub struct ExpGolombEncoder {
    buffer: Vec<u8>,
    bit_position: u8,
}

impl ExpGolombEncoder {
    pub fn encode_ue(&mut self, value: u32);
    pub fn encode_se(&mut self, value: i32);
    pub fn finish(self) -> Vec<u8>;
}
```

### 2. SPS生成器

```rust
pub struct SPSGenerator {
    profile_idc: u8,
    level_idc: u8,
    width: u32,
    height: u32,
}

impl SPSGenerator {
    pub fn generate(&self) -> Vec<u8>;
}
```

### 3. PPS生成器

```rust
pub struct PPSGenerator {
    sps_id: u8,
}

impl PPSGenerator {
    pub fn generate(&self) -> Vec<u8>;
}
```

### 4. 切片编码器

```rust
pub struct SliceEncoder {
    slice_type: SliceType,
    frame_num: u32,
}

impl SliceEncoder {
    pub fn encode_slice_header(&self) -> Vec<u8>;
    pub fn encode_macroblock_data(&self, yuv_data: &[u8]) -> Vec<u8>;
}
```

## 数据模型

### SPS结构
```
- profile_idc: u8 = 66 (Baseline Profile)
- constraint_set_flags: u8 = 0x80
- level_idc: u8 = 31 (Level 3.1)
- seq_parameter_set_id: UE = 0
- pic_width_in_mbs_minus1: UE
- pic_height_in_map_units_minus1: UE
- frame_mbs_only_flag: u1 = 1
- direct_8x8_inference_flag: u1 = 1
- frame_cropping_flag: u1 = 0
- vui_parameters_present_flag: u1 = 0
```

### PPS结构
```
- pic_parameter_set_id: UE = 0
- seq_parameter_set_id: UE = 0
- entropy_coding_mode_flag: u1 = 0
- pic_order_present_flag: u1 = 0
- num_slice_groups_minus1: UE = 0
- num_ref_idx_l0_active_minus1: UE = 0
- num_ref_idx_l1_active_minus1: UE = 0
- weighted_pred_flag: u1 = 0
- weighted_bipred_idc: u2 = 0
- pic_init_qp_minus26: SE = 0
- pic_init_qs_minus26: SE = 0
- chroma_qp_index_offset: SE = 0
- deblocking_filter_control_present_flag: u1 = 0
- constrained_intra_pred_flag: u1 = 0
- redundant_pic_cnt_present_flag: u1 = 0
```

## 正确性属性

*正确性属性是系统应该满足的特征或行为，作为人类可读规范和机器可验证正确性保证之间的桥梁。*

### 属性1: UE编码正确性
*对于任何*非负整数值，UE编码后再解码应该得到原始值
**验证: 需求3.1, 3.3, 3.4**

### 属性2: SE编码正确性  
*对于任何*有符号整数值，SE编码后再解码应该得到原始值
**验证: 需求3.2, 3.4**

### 属性3: SPS格式兼容性
*对于任何*有效的图像尺寸，生成的SPS应该能被标准H.264解码器解析
**验证: 需求1.1, 1.4, 1.5**

### 属性4: PPS引用一致性
*对于任何*PPS，其引用的SPS ID应该对应已生成的SPS
**验证: 需求2.2, 2.5**

### 属性5: 帧数据完整性
*对于任何*生成的帧，应该包含有效的切片头和宏块数据
**验证: 需求4.1, 4.2, 4.4, 4.5**

### 属性6: 播放器兼容性
*对于任何*生成的H.264文件，主流播放器应该能够正确解析和播放
**验证: 需求5.1, 5.2, 5.4**

## 错误处理

### 编码错误
- **UE编码溢出**: 限制输入值范围
- **位操作错误**: 使用安全的位操作函数
- **缓冲区溢出**: 预分配足够的缓冲区空间

### 格式错误
- **参数超范围**: 验证所有参数在H.264标准范围内
- **引用错误**: 确保PPS正确引用SPS
- **结构不完整**: 包含所有必要的H.264字段

## 测试策略

### 单元测试
- UE/SE编码解码往返测试
- SPS/PPS生成格式验证
- 切片头编码正确性测试

### 集成测试
- 完整H.264流生成测试
- ffprobe格式验证测试
- VLC播放兼容性测试

### 属性测试
- 随机输入的编码正确性测试
- 不同分辨率的SPS生成测试
- 各种帧类型的编码测试

使用**fast_check**作为属性测试库，每个测试运行100次迭代以确保可靠性。

## 实现计划

### 阶段1: 修复基础编码
1. 重写UE/SE编码函数
2. 实现正确的位操作
3. 添加编码验证测试

### 阶段2: 修复SPS/PPS
1. 重写SPS生成器
2. 重写PPS生成器  
3. 确保格式完全兼容

### 阶段3: 修复帧数据
1. 添加切片头编码
2. 改进宏块数据格式
3. 确保帧结构正确

### 阶段4: 验证和测试
1. 运行完整测试套件
2. 验证VLC播放兼容性
3. 性能优化和文档更新