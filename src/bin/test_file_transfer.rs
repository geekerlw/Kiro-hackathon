use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error};
use video_streaming_uploader::mock_server::{MockPlatformServer, PlaybackCommand};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting automated file transfer test...");

    // 连接到已运行的服务器（假设在8081端口）
    let server_addr: SocketAddr = "127.0.0.1:8082".parse()?;
    
    // 创建测试服务器
    let mut server = MockPlatformServer::new()?;
    server.start(server_addr).await?;
    
    info!("Test server started on {}", server_addr);
    
    // 等待连接建立
    sleep(Duration::from_secs(2)).await;
    
    // 获取活跃会话
    let sessions = server.get_all_sessions().await;
    if sessions.is_empty() {
        error!("No active sessions found. Make sure client is connected.");
        return Ok(());
    }
    
    let (session_id, _) = sessions[0];
    info!("Found active session: {}", session_id);
    
    // 测试1: 查询可用文件
    info!("Test 1: Querying available files...");
    match server.query_client_files(session_id, None).await {
        Ok(_) => info!("✓ File query sent successfully"),
        Err(e) => error!("✗ File query failed: {}", e),
    }
    
    sleep(Duration::from_secs(1)).await;
    
    // 测试2: 请求文件上传
    info!("Test 2: Requesting file upload...");
    match server.request_file_upload(session_id, "test_videos/test_video_small.mp4".to_string()).await {
        Ok(_) => info!("✓ File upload request sent successfully"),
        Err(e) => error!("✗ File upload request failed: {}", e),
    }
    
    sleep(Duration::from_secs(3)).await;
    
    // 测试3: 播放控制 - 跳转
    info!("Test 3: Testing seek control...");
    match server.send_playback_control(session_id, PlaybackCommand::Seek { position: 30.0 }).await {
        Ok(_) => info!("✓ Seek command sent successfully"),
        Err(e) => error!("✗ Seek command failed: {}", e),
    }
    
    sleep(Duration::from_secs(1)).await;
    
    // 测试4: 播放控制 - 倍速
    info!("Test 4: Testing rate control...");
    match server.send_playback_control(session_id, PlaybackCommand::SetRate { rate: 2.0 }).await {
        Ok(_) => info!("✓ Rate change command sent successfully"),
        Err(e) => error!("✗ Rate change command failed: {}", e),
    }
    
    sleep(Duration::from_secs(1)).await;
    
    // 测试5: 播放控制 - 暂停
    info!("Test 5: Testing pause control...");
    match server.send_playback_control(session_id, PlaybackCommand::Pause).await {
        Ok(_) => info!("✓ Pause command sent successfully"),
        Err(e) => error!("✗ Pause command failed: {}", e),
    }
    
    sleep(Duration::from_secs(1)).await;
    
    // 测试6: 播放控制 - 恢复
    info!("Test 6: Testing resume control...");
    match server.send_playback_control(session_id, PlaybackCommand::Resume).await {
        Ok(_) => info!("✓ Resume command sent successfully"),
        Err(e) => error!("✗ Resume command failed: {}", e),
    }
    
    sleep(Duration::from_secs(2)).await;
    
    // 测试7: 查看统计信息
    info!("Test 7: Checking session statistics...");
    match server.get_session_stats(session_id).await {
        Some(stats) => {
            info!("✓ Session statistics:");
            info!("  Status: {:?}", stats.status);
            info!("  Total segments: {}", stats.total_segments);
            info!("  Video segments: {}", stats.video_segments);
            info!("  Audio segments: {}", stats.audio_segments);
            info!("  Requested files: {:?}", stats.requested_files);
        }
        None => error!("✗ Failed to get session statistics"),
    }
    
    info!("Automated file transfer test completed!");
    info!("Check the client logs to see the actual file transfer progress.");
    
    Ok(())
}