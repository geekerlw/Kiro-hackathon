use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use clap::{Arg, Command};
use tracing::{info, error, Level};
use tracing_subscriber;
use uuid::Uuid;

use video_streaming_uploader::mock_server::{MockPlatformServer, FileInfo, PlaybackCommand};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // 解析命令行参数
    let matches = Command::new("Mock Platform Server")
        .version("1.0")
        .author("Video Streaming Team")
        .about("Mock platform server for testing video streaming uploads")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("Server listening port")
                .default_value("8080")
        )
        .arg(
            Arg::new("host")
                .short('H')
                .long("host")
                .value_name("HOST")
                .help("Server listening host")
                .default_value("127.0.0.1")
        )
        .get_matches();

    let port: u16 = matches.get_one::<String>("port")
        .unwrap()
        .parse()
        .expect("Invalid port number");
    
    let host: IpAddr = matches.get_one::<String>("host")
        .unwrap()
        .parse()
        .expect("Invalid host address");

    let bind_addr = SocketAddr::new(host, port);

    // 创建并启动模拟服务器
    let mut server = MockPlatformServer::new()?;
    
    info!("Starting mock platform server...");
    server.start(bind_addr).await?;

    // 启动交互式控制台
    start_interactive_console(server).await?;

    Ok(())
}



/// 获取会话ID，如果解析失败则使用第一个可用的会话
async fn get_session_id(server: &MockPlatformServer, session_str: &str) -> Option<Uuid> {
    match Uuid::parse_str(session_str) {
        Ok(id) => Some(id),
        Err(_) => {
            let sessions = server.get_all_sessions().await;
            if let Some((first_session_id, _)) = sessions.first() {
                info!("Using first available session ID: {}", first_session_id);
                Some(*first_session_id)
            } else {
                error!("No active sessions available. Please wait for a client to connect.");
                None
            }
        }
    }
}

/// 启动交互式控制台
async fn start_interactive_console(server: MockPlatformServer) -> Result<(), Box<dyn std::error::Error>> {
    info!("Mock server is running. Available commands:");
    info!("  list - List all active sessions");
    info!("  query [session_id] [filter] - Query available files from client");
    info!("  request <file_path> [session_id] - Request file upload from client");
    info!("  seek <position> [session_id] - Send seek command to client");
    info!("  rate <rate> [session_id] - Change playback rate");
    info!("  pause [session_id] - Pause upload");
    info!("  resume [session_id] - Resume upload");
    info!("  stop [session_id] - Stop upload");
    info!("  stats [session_id] - Show session statistics");
    info!("  finalize [session_id] - Finalize file reception and generate report");
    info!("  quit - Exit server");
    info!("");
    info!("Note: If session_id is not provided, the first available session will be used.");

    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "query" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        let filter = if parts.len() >= 3 {
                            Some(parts[2].to_string())
                        } else {
                            None
                        };
                        
                        match server.query_client_files(session_id, filter).await {
                            Ok(_) => info!("Queried file list from session {}", session_id),
                            Err(e) => error!("Failed to query files: {}", e),
                        }
                    }
                } else {
                    // 如果没有提供session_id，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        match server.query_client_files(*session_id, None).await {
                            Ok(_) => info!("Queried file list from session {}", session_id),
                            Err(e) => error!("Failed to query files: {}", e),
                        }
                    } else {
                        info!("Usage: query [session_id] [filter] - No active sessions available");
                    }
                }
            }

            "request" => {
                if parts.len() >= 3 {
                    let session_id = match Uuid::parse_str(parts[1]) {
                        Ok(id) => id,
                        Err(_) => {
                            // 如果不是有效的UUID，尝试使用第一个可用的会话
                            let sessions = server.get_all_sessions().await;
                            if let Some((first_session_id, _)) = sessions.first() {
                                info!("Using first available session ID: {}", first_session_id);
                                *first_session_id
                            } else {
                                error!("No active sessions available. Please wait for a client to connect.");
                                continue;
                            }
                        }
                    };
                    let file_path = parts[2].to_string();
                    
                    match server.request_file_upload(session_id, file_path.clone()).await {
                        Ok(_) => info!("Requested upload of {} for session {}", file_path, session_id),
                        Err(e) => error!("Failed to request upload: {}", e),
                    }
                } else if parts.len() == 2 {
                    // 如果只提供了文件路径，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        let file_path = parts[1].to_string();
                        match server.request_file_upload(*session_id, file_path.clone()).await {
                            Ok(_) => info!("Requested upload of {} for session {}", file_path, session_id),
                            Err(e) => error!("Failed to request upload: {}", e),
                        }
                    } else {
                        error!("No active sessions available. Please wait for a client to connect.");
                    }
                } else {
                    info!("Usage: request <session_id> <file_path> or request <file_path>");
                }
            }

            "seek" => {
                if parts.len() >= 3 {
                    if let (Some(session_id), Ok(position)) = (get_session_id(&server, parts[1]).await, parts[2].parse::<f64>()) {
                        let command = PlaybackCommand::Seek { position };
                        match server.send_playback_control(session_id, command).await {
                            Ok(_) => info!("Sent seek command to session {}: position {}", session_id, position),
                            Err(e) => error!("Failed to send seek command: {}", e),
                        }
                    } else {
                        info!("Invalid position");
                    }
                } else if parts.len() == 2 {
                    // 如果只提供了位置，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        if let Ok(position) = parts[1].parse::<f64>() {
                            let command = PlaybackCommand::Seek { position };
                            match server.send_playback_control(*session_id, command).await {
                                Ok(_) => info!("Sent seek command to session {}: position {}", session_id, position),
                                Err(e) => error!("Failed to send seek command: {}", e),
                            }
                        } else {
                            info!("Invalid position");
                        }
                    } else {
                        error!("No active sessions available");
                    }
                } else {
                    info!("Usage: seek <session_id> <position> or seek <position>");
                }
            }

            "rate" => {
                if parts.len() >= 3 {
                    if let (Some(session_id), Ok(rate)) = (get_session_id(&server, parts[1]).await, parts[2].parse::<f64>()) {
                        let command = PlaybackCommand::SetRate { rate };
                        match server.send_playback_control(session_id, command).await {
                            Ok(_) => info!("Sent rate change to session {}: rate {}", session_id, rate),
                            Err(e) => error!("Failed to send rate command: {}", e),
                        }
                    } else {
                        info!("Invalid rate");
                    }
                } else if parts.len() == 2 {
                    // 如果只提供了倍速，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        if let Ok(rate) = parts[1].parse::<f64>() {
                            let command = PlaybackCommand::SetRate { rate };
                            match server.send_playback_control(*session_id, command).await {
                                Ok(_) => info!("Sent rate change to session {}: rate {}", session_id, rate),
                                Err(e) => error!("Failed to send rate command: {}", e),
                            }
                        } else {
                            info!("Invalid rate");
                        }
                    } else {
                        error!("No active sessions available");
                    }
                } else {
                    info!("Usage: rate <session_id> <rate> or rate <rate>");
                }
            }

            "pause" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        let command = PlaybackCommand::Pause;
                        match server.send_playback_control(session_id, command).await {
                            Ok(_) => info!("Sent pause command to session {}", session_id),
                            Err(e) => error!("Failed to send pause command: {}", e),
                        }
                    }
                } else {
                    // 如果没有提供session_id，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        let command = PlaybackCommand::Pause;
                        match server.send_playback_control(*session_id, command).await {
                            Ok(_) => info!("Sent pause command to session {}", session_id),
                            Err(e) => error!("Failed to send pause command: {}", e),
                        }
                    } else {
                        info!("Usage: pause [session_id] - No active sessions available");
                    }
                }
            }

            "resume" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        let command = PlaybackCommand::Resume;
                        match server.send_playback_control(session_id, command).await {
                            Ok(_) => info!("Sent resume command to session {}", session_id),
                            Err(e) => error!("Failed to send resume command: {}", e),
                        }
                    }
                } else {
                    // 如果没有提供session_id，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        let command = PlaybackCommand::Resume;
                        match server.send_playback_control(*session_id, command).await {
                            Ok(_) => info!("Sent resume command to session {}", session_id),
                            Err(e) => error!("Failed to send resume command: {}", e),
                        }
                    } else {
                        info!("Usage: resume [session_id] - No active sessions available");
                    }
                }
            }

            "stop" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        let command = PlaybackCommand::Stop;
                        match server.send_playback_control(session_id, command).await {
                            Ok(_) => info!("Sent stop command to session {}", session_id),
                            Err(e) => error!("Failed to send stop command: {}", e),
                        }
                    }
                } else {
                    // 如果没有提供session_id，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        let command = PlaybackCommand::Stop;
                        match server.send_playback_control(*session_id, command).await {
                            Ok(_) => info!("Sent stop command to session {}", session_id),
                            Err(e) => error!("Failed to send stop command: {}", e),
                        }
                    } else {
                        info!("Usage: stop [session_id] - No active sessions available");
                    }
                }
            }

            "stats" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        match server.get_session_stats(session_id).await {
                            Some(stats) => {
                                info!("Session {} statistics:", session_id);
                                info!("  Status: {:?}", stats.status);
                                info!("  Total segments: {}", stats.total_segments);
                                info!("  Video segments: {}", stats.video_segments);
                                info!("  Audio segments: {}", stats.audio_segments);
                                info!("  Requested files: {:?}", stats.requested_files);
                                if let Some(current_file) = &stats.current_file {
                                    info!("  Current receiving: {}", current_file);
                                    info!("  Received size: {} bytes", stats.received_size);
                                    if let Some(output_path) = &stats.output_path {
                                        info!("  Output file: {:?}", output_path);
                                    }
                                }
                            }
                            None => info!("Session {} not found", session_id),
                        }
                    }
                } else {
                    // 如果没有提供session_id，显示所有会话的统计信息
                    let sessions = server.get_all_sessions().await;
                    if sessions.is_empty() {
                        info!("No active sessions");
                    } else {
                        info!("All session statistics:");
                        for (session_id, _) in sessions {
                            if let Some(stats) = server.get_session_stats(session_id).await {
                                info!("Session {}:", session_id);
                                info!("  Status: {:?}", stats.status);
                                info!("  Total segments: {}", stats.total_segments);
                                info!("  Video segments: {}", stats.video_segments);
                                info!("  Audio segments: {}", stats.audio_segments);
                                info!("  Requested files: {:?}", stats.requested_files);
                                if let Some(current_file) = &stats.current_file {
                                    info!("  Current receiving: {}", current_file);
                                    info!("  Received size: {} bytes", stats.received_size);
                                    if let Some(output_path) = &stats.output_path {
                                        info!("  Output file: {:?}", output_path);
                                    }
                                }
                                info!("");
                            }
                        }
                    }
                }
            }

            "finalize" => {
                if parts.len() >= 2 {
                    if let Some(session_id) = get_session_id(&server, parts[1]).await {
                        match server.finalize_file_reception(session_id).await {
                            Ok(_) => info!("Finalized file reception for session {}", session_id),
                            Err(e) => error!("Failed to finalize file reception: {}", e),
                        }
                    }
                } else {
                    // 如果没有提供session_id，使用第一个可用的会话
                    let sessions = server.get_all_sessions().await;
                    if let Some((session_id, _)) = sessions.first() {
                        match server.finalize_file_reception(*session_id).await {
                            Ok(_) => info!("Finalized file reception for session {}", session_id),
                            Err(e) => error!("Failed to finalize file reception: {}", e),
                        }
                    } else {
                        info!("Usage: finalize [session_id] - No active sessions available");
                    }
                }
            }

            "list" => {
                let sessions = server.get_all_sessions().await;
                if sessions.is_empty() {
                    info!("No active sessions");
                } else {
                    info!("Active sessions:");
                    for (session_id, status) in sessions {
                        info!("  {} - {:?}", session_id, status);
                    }
                }
            }



            "quit" | "exit" => {
                info!("Shutting down server...");
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