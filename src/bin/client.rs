use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use clap::{Arg, Command};
use tracing::{info, error, Level};
use tracing_subscriber;

use video_streaming_uploader::on_demand_uploader::OnDemandUploader;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // 解析命令行参数
    let matches = Command::new("Video Streaming Client")
        .version("1.0")
        .author("Video Streaming Team")
        .about("On-demand video streaming upload client")
        .arg(
            Arg::new("server")
                .short('s')
                .long("server")
                .value_name("SERVER")
                .help("Platform server address")
                .default_value("127.0.0.1:8080")
        )
        .arg(
            Arg::new("files")
                .short('f')
                .long("files")
                .value_name("FILES")
                .help("Comma-separated list of video files to register")
                .required(false)
        )
        .arg(
            Arg::new("directory")
                .short('d')
                .long("directory")
                .value_name("DIRECTORY")
                .help("Directory to scan for video files")
                .required(false)
        )
        .get_matches();

    let server_addr: SocketAddr = matches.get_one::<String>("server")
        .unwrap()
        .parse()
        .expect("Invalid server address");

    // 创建按需上传管理器
    let mut uploader = OnDemandUploader::new();

    info!("Starting video streaming client...");
    info!("Connecting to platform server at: {}", server_addr);

    // 连接到平台服务器
    uploader.connect_to_platform(server_addr).await?;
    info!("Successfully connected to platform server");

    // 注册本地文件
    if let Some(files_str) = matches.get_one::<String>("files") {
        let file_paths: Vec<&str> = files_str.split(',').collect();
        for file_path in file_paths {
            let path = PathBuf::from(file_path.trim());
            match uploader.register_local_file(path.clone()).await {
                Ok(_) => info!("Registered file: {:?}", path),
                Err(e) => error!("Failed to register file {:?}: {}", path, e),
            }
        }
    }

    // 扫描目录中的视频文件
    if let Some(directory) = matches.get_one::<String>("directory") {
        let dir_path = PathBuf::from(directory);
        if dir_path.is_dir() {
            info!("Scanning directory for video files: {:?}", dir_path);
            scan_directory_for_videos(&uploader, &dir_path).await?;
        } else {
            error!("Directory not found: {:?}", dir_path);
        }
    }

    // 如果没有指定文件或目录，注册一些默认的测试文件
    if matches.get_one::<String>("files").is_none() && matches.get_one::<String>("directory").is_none() {
        info!("No files specified, registering default test files...");
        register_default_test_files(&uploader).await;
    }

    // 显示已注册的文件
    let available_files = uploader.get_available_files().await;
    info!("Available files for streaming:");
    for file in &available_files {
        info!("  - {}", file);
    }

    if available_files.is_empty() {
        info!("No files available for streaming. The client will wait for platform requests.");
    }

    // 启动交互式控制台
    start_interactive_console(uploader).await?;

    Ok(())
}

/// 扫描目录中的视频文件
async fn scan_directory_for_videos(
    uploader: &OnDemandUploader,
    dir_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries = tokio::fs::read_dir(dir_path).await?;
    
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        
        if path.is_file() {
            if let Some(extension) = path.extension() {
                let ext = extension.to_string_lossy().to_lowercase();
                if matches!(ext.as_str(), "mp4" | "h264" | "avi" | "mov" | "mkv") {
                    match uploader.register_local_file(path.clone()).await {
                        Ok(_) => info!("Registered video file: {:?}", path),
                        Err(e) => error!("Failed to register video file {:?}: {}", path, e),
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// 注册默认测试文件
async fn register_default_test_files(uploader: &OnDemandUploader) {
    let test_files = vec![
        "sample_video_1.mp4",
        "sample_video_2.mp4", 
        "test_stream.h264",
    ];

    for file_path in test_files {
        let path = PathBuf::from(file_path);
        // 注意：这些文件可能不存在，所以注册会失败
        // 这只是为了演示目的
        match uploader.register_local_file(path.clone()).await {
            Ok(_) => info!("Registered test file: {:?}", path),
            Err(_) => info!("Test file not found (this is expected): {:?}", path),
        }
    }
}

/// 启动交互式控制台
async fn start_interactive_console(uploader: OnDemandUploader) -> Result<(), Box<dyn std::error::Error>> {
    info!("Client is ready and waiting for platform requests.");
    info!("Available commands:");
    info!("  status - Show client status");
    info!("  sessions - List active upload sessions");
    info!("  files - List registered files");
    info!("  timeline <file_path> - Show timeline info for a file");
    info!("  cache - Show timeline cache statistics");
    info!("  register <file_path> - Register a new file");
    info!("  regenerate <file_path> - Regenerate timeline for a file");
    info!("  quit - Exit client");
    info!("");

    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "status" => {
                let available_files = uploader.get_available_files().await;
                let active_sessions = uploader.get_active_sessions().await;
                
                info!("Client Status:");
                info!("  Registered files: {}", available_files.len());
                info!("  Active sessions: {}", active_sessions.len());
                info!("  Connection: Connected to platform server");
            }

            "sessions" => {
                let active_sessions = uploader.get_active_sessions().await;
                
                if active_sessions.is_empty() {
                    info!("No active upload sessions");
                } else {
                    info!("Active upload sessions:");
                    for session in active_sessions {
                        info!("  Session {}: {} ({:?})", 
                              session.id, 
                              session.file_path, 
                              session.status);
                        info!("    Progress: {}/{} segments", 
                              session.uploaded_segments, 
                              session.total_segments);
                        info!("    Position: {:.2}s, Rate: {:.1}x", 
                              session.current_position, 
                              session.playback_rate);
                    }
                }
            }

            "files" => {
                let available_files = uploader.get_available_files().await;
                
                if available_files.is_empty() {
                    info!("No files registered");
                } else {
                    info!("Registered files:");
                    for file in &available_files {
                        if let Some(file_info) = uploader.get_file_info(file).await {
                            let timeline_status = if file_info.timeline_data.is_some() {
                                "✅ Timeline available"
                            } else {
                                "❌ No timeline"
                            };
                            info!("  - {} ({:.2}s) - {}", 
                                  file, 
                                  file_info.duration.unwrap_or(0.0),
                                  timeline_status);
                        } else {
                            info!("  - {}", file);
                        }
                    }
                }
            }

            "timeline" => {
                if parts.len() >= 2 {
                    let file_path = parts[1];
                    if let Some(timeline) = uploader.get_file_timeline(file_path).await {
                        info!("Timeline for: {}", file_path);
                        info!("  Duration: {:.2} seconds", timeline.total_duration);
                        info!("  Keyframes: {}", timeline.keyframes.len());
                        info!("  Generated: {}", timeline.generated_at);
                        info!("  FFmpeg version: {}", timeline.ffmpeg_version);
                        
                        if !timeline.keyframes.is_empty() {
                            let avg_interval = timeline.total_duration / timeline.keyframes.len() as f64;
                            info!("  Average keyframe interval: {:.2}s", avg_interval);
                            
                            info!("  First 5 keyframes:");
                            for (i, keyframe) in timeline.keyframes.iter().take(5).enumerate() {
                                info!("    {}. {:.2}s @ offset {} ({} bytes)", 
                                      i + 1, keyframe.timestamp, keyframe.file_offset, keyframe.frame_size);
                            }
                            
                            if timeline.keyframes.len() > 5 {
                                info!("    ... and {} more keyframes", timeline.keyframes.len() - 5);
                            }
                        }
                    } else {
                        error!("No timeline data found for: {}", file_path);
                    }
                } else {
                    info!("Usage: timeline <file_path>");
                }
            }

            "cache" => {
                let stats = uploader.get_timeline_cache_stats().await;
                info!("Timeline Cache Statistics:");
                info!("  Cached files: {}", stats.cached_files);
                info!("  Cache size: {:.2} MB / {:.2} MB", 
                      stats.cache_size_bytes as f64 / 1024.0 / 1024.0,
                      stats.cache_limit_bytes as f64 / 1024.0 / 1024.0);
                info!("  Cache usage: {:.1}%", stats.cache_usage_percent);
            }

            "register" => {
                if parts.len() >= 2 {
                    let file_path = PathBuf::from(parts[1]);
                    info!("Registering file: {:?}", file_path);
                    match uploader.register_local_file(file_path.clone()).await {
                        Ok(_) => {
                            info!("Successfully registered file: {:?}", file_path);
                            // 显示时间轴信息
                            let file_key = file_path.to_string_lossy().to_string();
                            if let Some(timeline) = uploader.get_file_timeline(&file_key).await {
                                info!("  Timeline generated: {:.2}s duration, {} keyframes", 
                                      timeline.total_duration, timeline.keyframes.len());
                            }
                        }
                        Err(e) => error!("Failed to register file {:?}: {}", file_path, e),
                    }
                } else {
                    info!("Usage: register <file_path>");
                }
            }

            "regenerate" => {
                if parts.len() >= 2 {
                    let file_path = parts[1];
                    info!("Regenerating timeline for: {}", file_path);
                    match uploader.regenerate_timeline(file_path).await {
                        Ok(_) => {
                            info!("Timeline regenerated successfully");
                            if let Some(timeline) = uploader.get_file_timeline(file_path).await {
                                info!("  New timeline: {:.2}s duration, {} keyframes", 
                                      timeline.total_duration, timeline.keyframes.len());
                            }
                        }
                        Err(e) => error!("Failed to regenerate timeline: {}", e),
                    }
                } else {
                    info!("Usage: regenerate <file_path>");
                }
            }

            "quit" | "exit" => {
                info!("Shutting down client...");
                break;
            }

            _ => {
                info!("Unknown command: {}. Type 'quit' to exit.", parts[0]);
            }
        }
    }

    Ok(())
}

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::io::AsyncRead;

trait AsyncBufReadExtCompat {
    async fn next_line(&mut self) -> Result<Option<String>, std::io::Error>;
}

impl<R: AsyncRead + Unpin> AsyncBufReadExtCompat for BufReader<R> {
    async fn next_line(&mut self) -> Result<Option<String>, std::io::Error> {
        let mut line = String::new();
        match self.read_line(&mut line).await? {
            0 => Ok(None),
            _ => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Ok(Some(line))
            }
        }
    }
}