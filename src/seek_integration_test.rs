#[cfg(test)]
mod tests {
    use crate::controller::{DefaultPlaybackController, PlaybackController};
    use crate::file_reader::{DefaultFileStreamReader, FileStreamReader};
    use crate::types::{KeyframeIndex, KeyframeEntry, IndexOptimizationStrategy, FrameType};
    use std::path::Path;

    fn create_comprehensive_keyframe_index() -> KeyframeIndex {
        KeyframeIndex {
            entries: vec![
                KeyframeEntry {
                    timestamp: 0.0,
                    file_offset: 0,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 1.0,
                    file_offset: 30720,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 2.0,
                    file_offset: 61440,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 3.0,
                    file_offset: 92160,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 4.0,
                    file_offset: 122880,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
            ],
            total_duration: 5.0,
            index_precision: 0.033,
            memory_optimized: false,
            optimization_strategy: IndexOptimizationStrategy::Full,
            memory_usage: 160,
        }
    }

    #[tokio::test]
    async fn test_seek_integration_with_keyframe_index() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_comprehensive_keyframe_index();

        // Test seeking to exact keyframe positions
        let result = controller.seek_to_keyframe(2.0, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 2.0);
        assert_eq!(seek_result.actual_time, 2.0);
        assert_eq!(seek_result.keyframe_offset, 61440);
        assert_eq!(seek_result.precision_achieved, 1.0); // Perfect precision
        assert_eq!(controller.get_current_position(), 2.0);

        // Test seeking to non-keyframe positions (should align to previous keyframe)
        let result = controller.seek_to_keyframe(2.5, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 2.5);
        assert_eq!(seek_result.actual_time, 2.0); // Aligned to previous keyframe
        assert_eq!(seek_result.keyframe_offset, 61440);
        assert!(seek_result.precision_achieved < 1.0); // Not perfect precision
        assert_eq!(controller.get_current_position(), 2.0);
    }

    #[tokio::test]
    async fn test_seek_with_alignment_integration() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_comprehensive_keyframe_index();

        // Test alignment functionality
        let result = controller.seek_with_alignment(1.7, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 1.7);
        assert_eq!(seek_result.actual_time, 1.0); // Should align to previous keyframe
        assert_eq!(seek_result.keyframe_offset, 30720);
        assert_eq!(controller.get_current_position(), 1.0);

        // Test seeking to exact keyframe with alignment
        let result = controller.seek_with_alignment(3.0, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 3.0);
        assert_eq!(seek_result.actual_time, 3.0);
        assert_eq!(seek_result.precision_achieved, 1.0); // Perfect precision for exact keyframe
    }

    #[test]
    fn test_find_optimal_keyframe_strategies() {
        let controller = DefaultPlaybackController::new();
        let mut index = create_comprehensive_keyframe_index();

        // Test Full strategy
        index.optimization_strategy = IndexOptimizationStrategy::Full;
        let keyframe = controller.find_optimal_keyframe(2.3, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 2.0);

        // Test Sparse strategy
        index.optimization_strategy = IndexOptimizationStrategy::Sparse;
        let keyframe = controller.find_optimal_keyframe(2.3, &index);
        assert!(keyframe.is_some());

        // Test Adaptive strategy with low memory usage
        index.optimization_strategy = IndexOptimizationStrategy::Adaptive;
        index.memory_usage = 32; // Low memory usage
        let keyframe = controller.find_optimal_keyframe(2.3, &index);
        assert!(keyframe.is_some());

        // Test Adaptive strategy with high memory usage
        index.memory_usage = 1024; // High memory usage
        let keyframe = controller.find_optimal_keyframe(2.3, &index);
        assert!(keyframe.is_some());

        // Test Hierarchical strategy
        index.optimization_strategy = IndexOptimizationStrategy::Hierarchical;
        let keyframe = controller.find_optimal_keyframe(2.3, &index);
        assert!(keyframe.is_some());
    }

    #[test]
    fn test_keyframe_index_validation() {
        let controller = DefaultPlaybackController::new();
        
        // Test valid index
        let valid_index = create_comprehensive_keyframe_index();
        assert!(controller.validate_keyframe_index(&valid_index).is_ok());

        // Test empty index
        let mut empty_index = create_comprehensive_keyframe_index();
        empty_index.entries.clear();
        assert!(controller.validate_keyframe_index(&empty_index).is_err());

        // Test unsorted index
        let mut unsorted_index = create_comprehensive_keyframe_index();
        unsorted_index.entries.swap(0, 2); // Make it unsorted
        assert!(controller.validate_keyframe_index(&unsorted_index).is_err());

        // Test invalid precision
        let mut invalid_precision_index = create_comprehensive_keyframe_index();
        invalid_precision_index.index_precision = -0.1;
        assert!(controller.validate_keyframe_index(&invalid_precision_index).is_err());

        // Test inconsistent duration
        let mut invalid_duration_index = create_comprehensive_keyframe_index();
        invalid_duration_index.total_duration = 2.0; // Less than last keyframe timestamp
        assert!(controller.validate_keyframe_index(&invalid_duration_index).is_err());
    }

    #[tokio::test]
    async fn test_seek_error_handling() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_comprehensive_keyframe_index();

        // Test seeking to negative position
        let result = controller.seek_to_keyframe(-1.0, &index).await;
        assert!(result.is_err());

        // Test seeking beyond duration
        let result = controller.seek_to_keyframe(10.0, &index).await;
        assert!(result.is_err());

        // Test seeking with empty index
        let mut empty_index = create_comprehensive_keyframe_index();
        empty_index.entries.clear();
        let result = controller.seek_to_keyframe(2.0, &empty_index).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_seek_performance_characteristics() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_comprehensive_keyframe_index();

        // Test multiple seeks to measure performance consistency
        let mut execution_times = Vec::new();
        
        for i in 0..10 {
            let seek_time = i as f64 * 0.5;
            if seek_time <= index.total_duration {
                let result = controller.seek_to_keyframe(seek_time, &index).await;
                assert!(result.is_ok());
                
                let seek_result = result.unwrap();
                execution_times.push(seek_result.execution_time);
                
                // Verify seek was successful
                assert!(seek_result.precision_achieved >= 0.0);
                assert!(seek_result.precision_achieved <= 1.0);
                assert!(seek_result.execution_time.as_nanos() >= 0);
            }
        }

        // Verify all seeks completed in reasonable time (less than 1ms)
        for time in execution_times {
            assert!(time.as_nanos() >= 0, "Invalid execution time: {:?}", time);
            assert!(time.as_micros() < 1000, "Seek took too long: {:?}", time);
        }
    }

    #[test]
    fn test_seek_info_tracking() {
        let mut controller = DefaultPlaybackController::new();
        
        // Initially no seek info
        assert!(controller.get_seek_info().is_none());

        // After regular seek
        let _ = tokio_test::block_on(controller.seek(3.5));
        let seek_info = controller.get_seek_info();
        assert!(seek_info.is_some());
        let (last_seek, current_pos) = seek_info.unwrap();
        assert_eq!(last_seek, 3.5);
        assert_eq!(current_pos, 3.5);

        // After keyframe seek
        let index = create_comprehensive_keyframe_index();
        let _ = tokio_test::block_on(controller.seek_to_keyframe(2.3, &index));
        let seek_info = controller.get_seek_info();
        assert!(seek_info.is_some());
        let (last_seek, current_pos) = seek_info.unwrap();
        assert_eq!(last_seek, 2.0); // Should be aligned position
        assert_eq!(current_pos, 2.0);
    }

    #[tokio::test]
    async fn test_integration_with_file_reader() {
        // This test verifies that the controller's seek functionality
        // works correctly with actual file operations
        let reader = DefaultFileStreamReader::new();
        
        // Try to find a test video file
        let test_files = [
            "test_videos/sample1.mp4",
            "test_videos/test_video_small.mp4",
        ];
        
        let mut video_file = None;
        for file_path in &test_files {
            if Path::new(file_path).exists() {
                video_file = Some(file_path);
                break;
            }
        }
        
        if let Some(video_path) = video_file {
            let mut file_handle = reader.open_file(Path::new(video_path)).await.unwrap();
            let index = reader.build_keyframe_index(&mut file_handle).await.unwrap();
            
            if !index.entries.is_empty() {
                let mut controller = DefaultPlaybackController::new();
                
                // Test seeking to middle of video
                let middle_time = index.total_duration / 2.0;
                let result = controller.seek_to_keyframe(middle_time, &index).await;
                assert!(result.is_ok());
                
                let seek_result = result.unwrap();
                assert!(seek_result.actual_time <= middle_time);
                assert!(seek_result.keyframe_offset > 0);
                assert!(seek_result.precision_achieved >= 0.0);
                
                // Verify controller state was updated
                assert_eq!(controller.get_current_position(), seek_result.actual_time);
            }
        }
    }
}