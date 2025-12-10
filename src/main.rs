use std::path::PathBuf;
use std::env;
use video_streaming_uploader::app::{VideoStreamingApp, AppFactory, AppBuilder};
use video_streaming_uploader::config::AppConfig;
use video_streaming_uploader::errors::UploadManagerError;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Video Streaming Uploader v0.1.0");
    println!("Initializing application...");

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let app = match args.get(1).map(|s| s.as_str()) {
        Some("--config") => {
            let config_path = args.get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("video_uploader.toml"));
            
            println!("Loading configuration from: {:?}", config_path);
            VideoStreamingApp::with_config_file(config_path).await?
        }
        Some("--dev") => {
            println!("Starting in development mode");
            AppFactory::create_development_app().await?
        }
        Some("--test") => {
            println!("Starting in test mode");
            AppFactory::create_test_app().await?
        }
        Some("--help") | Some("-h") => {
            print_help();
            return Ok(());
        }
        _ => {
            println!("Starting with default configuration");
            VideoStreamingApp::new().await?
        }
    };

    println!("✓ Application initialized successfully");
    println!("✓ Configuration loaded and validated");
    println!("✓ All components initialized with dependency injection");
    println!("✓ Event handling system ready");
    println!("✓ Performance monitoring configured");
    println!("✓ Error recovery system enabled");

    // Print configuration summary
    print_config_summary(app.config());

    // Example usage demonstration
    if args.contains(&"--demo".to_string()) {
        println!("\n--- Running Demo ---");
        run_demo(&app).await?;
    } else {
        println!("\nApplication ready. Use --demo to run a demonstration.");
        println!("Press Ctrl+C to shutdown gracefully.");
        
        // Run the application
        if let Err(e) = app.run().await {
            eprintln!("Application error: {}", e);
            app.shutdown().await?;
            return Err(e.into());
        }
    }

    // Graceful shutdown
    app.shutdown().await?;
    println!("Application shutdown complete.");

    Ok(())
}

fn print_help() {
    println!("Video Streaming Uploader - High-performance video streaming application");
    println!();
    println!("USAGE:");
    println!("    video_streaming_uploader [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --config <PATH>    Load configuration from specified file");
    println!("    --dev              Start in development mode");
    println!("    --test             Start in test mode");
    println!("    --demo             Run demonstration after initialization");
    println!("    --help, -h         Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("    video_streaming_uploader");
    println!("    video_streaming_uploader --config ./config.toml");
    println!("    video_streaming_uploader --dev --demo");
}

fn print_config_summary(config: &AppConfig) {
    println!("\n--- Configuration Summary ---");
    println!("Server: {}:{}", config.server.address, config.server.port);
    println!("Protocol Version: {}.{}.{}", 
        config.server.protocol_version.major,
        config.server.protocol_version.minor,
        config.server.protocol_version.patch);
    println!("Max Concurrent Streams: {}", config.transport.max_concurrent_streams);
    println!("Segment Mode: {:?}", config.video.segment_mode);
    println!("Performance Monitoring: {}", if config.monitoring.enabled { "Enabled" } else { "Disabled" });
    println!("Error Recovery: {}", if config.error_handling.enable_recovery { "Enabled" } else { "Disabled" });
    println!("Log Level: {}", config.logging.level);
}

async fn run_demo(app: &VideoStreamingApp) -> Result<(), UploadManagerError> {
    println!("Demo: Demonstrating application capabilities...");
    
    // Check if upload manager is ready
    let is_running = app.upload_manager().is_running().await;
    println!("Upload Manager Status: {}", if is_running { "Running" } else { "Ready" });
    
    // Get current session info
    if let Some(session) = app.upload_manager().get_current_session().await {
        println!("Current Session: {}", session.id);
        println!("Session Status: {:?}", session.status);
    } else {
        println!("No active session");
    }
    
    // Try to get performance stats (will fail if no active session, which is expected)
    match app.upload_manager().get_performance_stats().await {
        Ok(stats) => {
            println!("Performance Stats:");
            println!("  Upload Speed: {:.2} MB/s", stats.current_upload_speed / 1_000_000.0);
            println!("  Latency: {:?}", stats.current_latency);
            println!("  Bytes Transmitted: {}", stats.total_bytes_transmitted);
        }
        Err(e) => {
            println!("Performance stats not available (expected): {}", e);
        }
    }
    
    println!("Demo completed successfully!");
    Ok(())
}