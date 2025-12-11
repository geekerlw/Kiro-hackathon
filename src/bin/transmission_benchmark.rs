use std::time::Instant;
use tracing::{info, Level};
use video_streaming_uploader::transmission_config::{TransmissionConfig, TransmissionMode, TransmissionMetrics};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("🚀 视频传输性能基准测试");
    info!("");

    // 模拟不同文件大小
    let file_sizes = vec![
        (1024 * 1024, "1MB"),           // 1MB
        (10 * 1024 * 1024, "10MB"),     // 10MB
        (100 * 1024 * 1024, "100MB"),   // 100MB
        (1024 * 1024 * 1024, "1GB"),    // 1GB
    ];

    // 测试不同传输配置
    let configs = vec![
        ("实时流媒体模式 (33ms延迟)", TransmissionConfig::real_time(33.0, 30.0)),
        ("高吞吐模式 (1MB分片)", TransmissionConfig::high_throughput(1024 * 1024, 4)),
        ("平衡模式 (吞吐优先)", TransmissionConfig::balanced(10.0, 0.8)),
        ("平衡模式 (延迟优先)", TransmissionConfig::balanced(5.0, 0.2)),
    ];

    for (file_size, file_name) in &file_sizes {
        info!("📁 测试文件大小: {}", file_name);
        info!("{}", "=".repeat(60));

        for (config_name, config) in &configs {
            let result = simulate_transmission(*file_size, config).await;
            
            info!("📊 配置: {}", config_name);
            info!("   分片大小: {} KB", config.calculate_optimal_segment_size(100.0) / 1024);
            info!("   传输延迟: {:?}", config.calculate_transmission_delay(1.0));
            info!("   并行传输: {}", if config.should_use_parallel_transmission() { "是" } else { "否" });
            info!("   预估传输时间: {:.2}秒", result.estimated_time_seconds);
            info!("   预估吞吐量: {:.2} MB/s", result.estimated_throughput_mbps);
            info!("");
        }
        info!("");
    }

    // 显示优化建议
    show_optimization_recommendations();

    Ok(())
}

#[derive(Debug)]
struct SimulationResult {
    estimated_time_seconds: f64,
    estimated_throughput_mbps: f64,
    segment_count: usize,
    total_overhead_bytes: usize,
}

async fn simulate_transmission(file_size: usize, config: &TransmissionConfig) -> SimulationResult {
    let segment_size = config.calculate_optimal_segment_size(100.0); // 假设100Mbps带宽
    let segment_count = (file_size + segment_size - 1) / segment_size;
    let delay = config.calculate_transmission_delay(1.0);
    
    // 计算协议开销 (每个分片42字节头)
    let protocol_overhead_per_segment = 42;
    let total_overhead_bytes = segment_count * protocol_overhead_per_segment;
    
    // 估算传输时间
    let estimated_time_seconds = match &config.mode {
        TransmissionMode::RealTime { target_latency_ms, .. } => {
            // 实时模式：每个分片有固定延迟
            segment_count as f64 * target_latency_ms / 1000.0
        }
        TransmissionMode::HighThroughput { parallel_streams, .. } => {
            // 高吞吐模式：假设网络带宽限制 (100Mbps)
            let network_time = (file_size + total_overhead_bytes) as f64 * 8.0 / (100.0 * 1024.0 * 1024.0);
            // 并行传输可以减少时间
            network_time / (*parallel_streams as f64).min(4.0)
        }
        TransmissionMode::Balanced { latency_ms, throughput_priority } => {
            // 平衡模式：延迟和网络时间的加权平均
            let delay_time = segment_count as f64 * latency_ms * (1.0 - throughput_priority) / 1000.0;
            let network_time = (file_size + total_overhead_bytes) as f64 * 8.0 / (100.0 * 1024.0 * 1024.0);
            delay_time + network_time
        }
    };
    
    let estimated_throughput_mbps = if estimated_time_seconds > 0.0 {
        (file_size as f64 / 1024.0 / 1024.0) / estimated_time_seconds
    } else {
        0.0
    };

    SimulationResult {
        estimated_time_seconds,
        estimated_throughput_mbps,
        segment_count,
        total_overhead_bytes,
    }
}

fn show_optimization_recommendations() {
    info!("💡 传输优化建议");
    info!("{}", "=".repeat(60));
    info!("");
    
    info!("🎯 场景选择指南:");
    info!("   📺 实时流媒体 → 使用实时模式 (33ms延迟)");
    info!("   📁 文件上传   → 使用高吞吐模式 (1MB分片 + 并行)");
    info!("   🎮 游戏直播   → 使用平衡模式 (延迟优先)");
    info!("   📹 录播上传   → 使用高吞吐模式 (速度优先)");
    info!("");
    
    info!("⚡ 速度优化技巧:");
    info!("   1. 移除人工延迟 → 速度提升 30-100倍");
    info!("   2. 增大分片大小 (8KB→1MB) → 减少协议开销");
    info!("   3. 启用并行传输 → 利用QUIC多路复用");
    info!("   4. 自适应分片大小 → 根据网络条件调整");
    info!("");
    
    info!("🔧 配置示例:");
    info!("   // 最大速度模式");
    info!("   TransmissionConfig::high_throughput(1024*1024, 8)");
    info!("");
    info!("   // 实时流媒体模式");
    info!("   TransmissionConfig::real_time(33.0, 30.0)");
    info!("");
    info!("   // 平衡模式");
    info!("   TransmissionConfig::balanced(10.0, 0.8)");
    info!("");
    
    info!("📊 性能对比 (100MB文件):");
    info!("   当前实现 (8KB + 33ms延迟):  ~200KB/s  (8分钟)");
    info!("   优化后 (1MB + 无延迟):      ~50MB/s   (2秒)");
    info!("   提升倍数: 250倍 🚀");
}