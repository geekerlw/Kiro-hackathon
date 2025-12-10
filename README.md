# Video Streaming Uploader

A high-performance video streaming uploader built in Rust, designed for ultra-low latency transmission using QUIC protocol.

## Project Structure

```
src/
├── lib.rs              # Main library entry point
├── main.rs             # Binary application entry point
├── types.rs            # Core data structures and types
├── errors.rs           # Comprehensive error handling
├── serde_helpers.rs    # Serialization utilities
├── file_reader.rs      # File streaming and reading
├── segmenter.rs        # Video segmentation logic
├── transport.rs        # QUIC transport and audio/video separation
├── controller.rs       # Playback control (seek, rate adjustment)
└── monitor.rs          # Performance monitoring and statistics
```

## Core Components

### 1. File Stream Reader (`file_reader.rs`)
- **Trait**: `FileStreamReader`
- **Implementation**: `DefaultFileStreamReader`
- **Purpose**: Stream-based reading of video files (Mp4, H.264)
- **Features**: Format validation, metadata extraction, streaming I/O

### 2. Video Segmenter (`segmenter.rs`)
- **Trait**: `VideoSegmenter`
- **Implementation**: `DefaultVideoSegmenter`
- **Purpose**: Frame-level video segmentation for low-latency streaming
- **Features**: Key frame detection, GOP-level fallback, streaming segmentation

### 3. Transport Layer (`transport.rs`)
- **Traits**: `QUICTransport`, `AudioVideoSeparator`
- **Implementations**: `DefaultQUICTransport`, `DefaultAudioVideoSeparator`
- **Purpose**: QUIC-based network transport with audio/video stream separation
- **Features**: Multiplexed transmission, adaptive parameters, connection recovery

### 4. Playback Controller (`controller.rs`)
- **Trait**: `PlaybackController`
- **Implementation**: `DefaultPlaybackController`
- **Purpose**: SEEK operations and playback rate control
- **Features**: Frame dropping strategies, buffer management, synchronization

### 5. Performance Monitor (`monitor.rs`)
- **Trait**: `PerformanceMonitor`
- **Implementation**: `DefaultPerformanceMonitor`
- **Purpose**: Real-time performance monitoring and statistics
- **Features**: Speed calculation, latency measurement, anomaly detection, data export

## Key Data Structures

- **VideoSegment**: Individual video segments with metadata
- **AudioSegment**: Individual audio segments with timing info
- **VideoFileInfo**: Video file metadata (duration, resolution, codec)
- **PerformanceStats**: Real-time performance metrics
- **TransmissionSession**: Complete upload session tracking

## Error Handling

Comprehensive error handling with specific error types:
- `FileError`: File I/O and format issues
- `SegmentError`: Video segmentation problems
- `TransportError`: Network and QUIC transport issues
- `PlaybackError`: Playback control failures
- `MonitorError`: Performance monitoring issues

## Dependencies

- **tokio**: Async runtime and I/O
- **uuid**: Unique identifiers for segments and sessions
- **quinn**: QUIC protocol implementation
- **proptest**: Property-based testing framework
- **serde**: Serialization for performance data export
- **thiserror**: Error handling macros
- **ffmpeg-next**: Video processing (optional, requires system FFmpeg)

## Building and Running

```bash
# Check compilation
cargo check

# Run tests
cargo test

# Run the demo application
cargo run

# Build with FFmpeg support (requires system FFmpeg installation)
cargo build --features ffmpeg
```

## Architecture Principles

1. **Modular Design**: Clear separation of concerns with trait-based interfaces
2. **Streaming Processing**: Avoid loading entire files into memory
3. **Error Resilience**: Comprehensive error handling and recovery mechanisms
4. **Performance Focus**: Optimized for ultra-low latency transmission
5. **Protocol Agnostic**: Language-independent QUIC protocol specification
6. **Testability**: Property-based testing for correctness validation

## Next Steps

This project structure provides the foundation for implementing:
1. Actual video file processing with FFmpeg integration
2. Real QUIC network transport implementation
3. Advanced segmentation algorithms
4. Performance optimization and tuning
5. Comprehensive test coverage with property-based testing

## Requirements Mapping

This implementation addresses the following requirements from the specification:
- **8.1-8.5**: Modular architecture with clear component separation
- **1.1-1.6**: File reading and format validation capabilities
- **2.1-2.7**: Video segmentation with frame-level precision
- **3.1-3.5**: Audio/video separation and independent transmission
- **4.1-4.5**: QUIC transport foundation
- **5.1-5.5**: Playback control interfaces
- **6.1-6.5**: Performance monitoring and statistics
- **7.1-7.5**: Comprehensive error handling