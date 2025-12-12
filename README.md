# Video Streaming Uploader

A high-performance video streaming uploader built in Rust, designed for ultra-low latency transmission using QUIC protocol with **enhanced seek functionality** and **seamless upload continuation**.

## 🆕 Latest Features

### Enhanced Seek Functionality
- **Precise Keyframe-Based Seeking** - Sub-second accuracy with automatic keyframe index construction
- **FFmpeg Command-Line Integration** - Uses system FFmpeg for reliable video parsing
- **Timeline File Generation** - JSON-based keyframe timeline caching for fast seek operations
- **Automatic Keyframe Alignment** - Ensures decode integrity for non-keyframe positions
- **Detailed Seek Results** - Comprehensive feedback on seek operations

### Complete Seek-Continue Workflow
- **Query File List** → **Request Upload** → **Seek to Position** → **Continue Upload**
- **Seamless Upload Continuation** - Automatic restart from new seek positions
- **Real-time Performance Monitoring** - 1+ Gbps transmission rates with detailed statistics
- **Integration with Playback Control** - Works seamlessly with rate changes and pause/resume

### Performance Achievements
- **79 Tests Passing** ✅ - Comprehensive test coverage
- **1+ Gbps Throughput** - High-performance transmission
- **Sub-second Seek Precision** - 0.031s precision with keyframe indexing
- **O(log n) Seek Performance** - Binary search algorithm for keyframe lookup

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

### 4. 🆕 Enhanced Playback Controller (`controller.rs`)
- **Trait**: `PlaybackController`
- **Implementation**: `DefaultPlaybackController`
- **Purpose**: Advanced SEEK operations and playback rate control
- **Features**: 
  - **Precise keyframe-based seeking** with sub-second accuracy
  - **Multiple optimization strategies** (Full, Sparse, Adaptive, Hierarchical)
  - **Automatic keyframe alignment** for decode integrity
  - **Detailed seek results** with precision feedback
  - Frame dropping strategies, buffer management, synchronization

### 5. Performance Monitor (`monitor.rs`)
- **Trait**: `PerformanceMonitor`
- **Implementation**: `DefaultPerformanceMonitor`
- **Purpose**: Real-time performance monitoring and statistics
- **Features**: Speed calculation, latency measurement, anomaly detection, data export

## Key Data Structures

### Core Structures
- **VideoSegment**: Individual video segments with metadata
- **AudioSegment**: Individual audio segments with timing info
- **VideoFileInfo**: Video file metadata (duration, resolution, codec)
- **PerformanceStats**: Real-time performance metrics
- **TransmissionSession**: Complete upload session tracking

### 🆕 Enhanced Seek Structures
- **KeyframeIndex**: Keyframe index with optimization strategies
- **KeyframeEntry**: Individual keyframe information (timestamp, offset, size)
- **SeekResult**: Detailed seek operation results with precision metrics
- **IndexOptimizationStrategy**: Enum for different indexing strategies

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
- **chrono**: Date and time handling for timeline files
- **FFmpeg**: Command-line tool for video parsing (system dependency)

## Building and Running

### Prerequisites

Install FFmpeg command-line tool:

**macOS:**
```bash
brew install ffmpeg
```

**Ubuntu/Debian:**
```bash
sudo apt install ffmpeg
```

**Windows:**
Download from [FFmpeg official site](https://ffmpeg.org/download.html) and add to PATH.

**Verify installation:**
```bash
ffmpeg -version
```

### Build and Test

```bash
# Check compilation
cargo check

# Run all tests
cargo test --lib

# Run specific seek tests
cargo test seek --lib

# 🆕 Run FFmpeg command-line integration demo
cargo run --example ffmpeg_cli_demo

# 🆕 Run enhanced seek demos
cargo run --example seek_and_continue_demo
cargo run --example enhanced_seek_client_server_demo
cargo run --example keyframe_index_demo

# Run the main application
cargo run --bin mock-server  # Start server
cargo run --bin client       # Start client

# Build project
cargo build
```

## Architecture Principles

1. **Modular Design**: Clear separation of concerns with trait-based interfaces
2. **Streaming Processing**: Avoid loading entire files into memory
3. **Error Resilience**: Comprehensive error handling and recovery mechanisms
4. **Performance Focus**: Optimized for ultra-low latency transmission
5. **Protocol Agnostic**: Language-independent QUIC protocol specification
6. **Testability**: Property-based testing for correctness validation

## 🆕 Quick Start

### Complete Seek-Continue Workflow Demo
```bash
# Run the complete workflow demonstration
cargo run --example seek_and_continue_demo
```

This demo shows:
1. **File List Query** - Server queries available files from client
2. **Upload Request** - Server requests specific file upload
3. **Keyframe Index Construction** - Automatic building of keyframe index (18 keyframes, 0.031s precision)
4. **Precise Seek Operations** - Multiple seek operations with sub-second accuracy
5. **Upload Continuation** - Automatic restart from new positions
6. **Performance Monitoring** - Real-time statistics (1+ Gbps throughput)

### Manual Testing
```bash
# Terminal 1: Start server
cargo run --bin mock-server

# Terminal 2: Start client with test files
cargo run --bin client -- --files "test_videos/sample1.mp4"

# In server console:
query                    # Query available files
request sample1.mp4      # Request file upload
seek 30.5               # Seek to 30.5 seconds
rate 2.0                # Set 2x playback rate
seek 15.0               # Seek to 15 seconds at 2x rate
stats                   # View performance statistics
```

## Next Steps

This project provides a complete implementation with:
1. ✅ **Enhanced seek functionality** with keyframe indexing
2. ✅ **Complete seek-continue workflow** 
3. ✅ **High-performance transmission** (1+ Gbps)
4. ✅ **Comprehensive test coverage** (79 tests passing)
5. ✅ **Real-time performance monitoring**

Future enhancements:
1. Additional video format support
2. Predictive keyframe caching
3. Network-aware seek optimization
4. Integration with video quality adaptation

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
- **🆕 9.1-9.5**: Enhanced seek functionality with keyframe positioning
- **🆕 10.1-10.3**: Complete seek-continue upload workflow

## Documentation

- **[USAGE.md](USAGE.md)** - Complete usage guide with enhanced seek features
- **[ENHANCED_SEEK_USAGE.md](ENHANCED_SEEK_USAGE.md)** - Detailed seek functionality documentation
- **[FFMPEG_CLI_USAGE.md](FFMPEG_CLI_USAGE.md)** - FFmpeg command-line integration guide
- **[PLATFORM_INTEGRATION_PROTOCOL.md](PLATFORM_INTEGRATION_PROTOCOL.md)** - Protocol specification