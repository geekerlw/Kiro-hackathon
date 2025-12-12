#[cfg(test)]
mod tests {
    use crate::controller::{DefaultPlaybackController, PlaybackController};
    use crate::types::{VideoSegment, SegmentMetadata};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn create_test_segment(timestamp: f64, duration: f64, is_key_frame: bool) -> VideoSegment {
        VideoSegment {
            id: Uuid::new_v4(),
            data: vec![0u8; 1024],
            timestamp,
            duration,
            frame_count: 30,
            is_key_frame,
            metadata: SegmentMetadata {
                frame_indices: vec![0, 1, 2],
                key_frame_positions: if is_key_frame { vec![0] } else { vec![] },
                encoding_params: HashMap::new(),
            },
        }
    }

    #[tokio::test]
    async fn test_seek_functionality() {
        let mut controller = DefaultPlaybackController::new();
        
        // Test valid seek
        let result = controller.seek(10.5).await;
        assert!(result.is_ok());
        assert_eq!(controller.get_current_position(), 10.5);
        
        // Test invalid seek (negative position)
        let result = controller.seek(-1.0).await;
        assert!(result.is_err());
        
        // Position should remain unchanged after invalid seek
        assert_eq!(controller.get_current_position(), 10.5);
    }

    #[tokio::test]
    async fn test_playback_rate_control() {
        let mut controller = DefaultPlaybackController::new();
        
        // Test valid playback rates
        assert!(controller.set_playback_rate(1.5).await.is_ok());
        assert_eq!(controller.get_playback_rate(), 1.5);
        
        assert!(controller.set_playback_rate(0.5).await.is_ok());
        assert_eq!(controller.get_playback_rate(), 0.5);
        
        // Test invalid playback rates
        assert!(controller.set_playback_rate(0.0).await.is_err());
        assert!(controller.set_playback_rate(-1.0).await.is_err());
        assert!(controller.set_playback_rate(11.0).await.is_err());
        
        // Rate should remain unchanged after invalid attempts
        assert_eq!(controller.get_playback_rate(), 0.5);
    }

    #[test]
    fn test_drop_frame_strategy() {
        let controller = DefaultPlaybackController::new();
        
        // Test normal speed (no dropping)
        let strategy = controller.get_drop_frame_strategy(1.0);
        assert!(!strategy.drop_b_frames);
        assert!(!strategy.drop_p_frames);
        assert!(!strategy.keep_key_frames_only);
        
        // Test moderate speed (drop B frames)
        let strategy = controller.get_drop_frame_strategy(1.5);
        assert!(strategy.drop_b_frames);
        assert!(!strategy.drop_p_frames);
        assert!(!strategy.keep_key_frames_only);
        assert!(strategy.adaptive_dropping);
        
        // Test high speed (drop B and P frames)
        let strategy = controller.get_drop_frame_strategy(3.0);
        assert!(strategy.drop_b_frames);
        assert!(strategy.drop_p_frames);
        assert!(!strategy.keep_key_frames_only);
        
        // Test very high speed (key frames only)
        let strategy = controller.get_drop_frame_strategy(8.0);
        assert!(strategy.drop_b_frames);
        assert!(strategy.drop_p_frames);
        assert!(strategy.keep_key_frames_only);
    }

    #[test]
    fn test_find_segment_at_position() {
        let controller = DefaultPlaybackController::new();
        
        let segments = vec![
            create_test_segment(0.0, 2.0, true),   // 0-2s
            create_test_segment(2.0, 2.0, false),  // 2-4s
            create_test_segment(4.0, 2.0, true),   // 4-6s
            create_test_segment(6.0, 2.0, false),  // 6-8s
        ];
        
        // Test exact matches
        assert_eq!(controller.find_segment_at_position(&segments, 1.0), Some(0));
        assert_eq!(controller.find_segment_at_position(&segments, 3.0), Some(1));
        assert_eq!(controller.find_segment_at_position(&segments, 5.0), Some(2));
        
        // Test boundary conditions
        assert_eq!(controller.find_segment_at_position(&segments, 0.0), Some(0));
        assert_eq!(controller.find_segment_at_position(&segments, 2.0), Some(1));
        
        // Test position beyond all segments
        assert_eq!(controller.find_segment_at_position(&segments, 10.0), Some(3));
        
        // Test position before all segments
        assert_eq!(controller.find_segment_at_position(&segments, -1.0), None);
    }

    #[test]
    fn test_transmission_queue_adjustment() {
        let controller = DefaultPlaybackController::new();
        
        let segments = vec![
            create_test_segment(0.0, 1.0, true),   // Key frame
            create_test_segment(1.0, 1.0, false),  // Non-key frame
            create_test_segment(2.0, 1.0, false),  // Non-key frame
            create_test_segment(3.0, 1.0, true),   // Key frame
            create_test_segment(4.0, 1.0, false),  // Non-key frame
        ];
        
        // Test normal speed (keep all frames)
        let adjusted = controller.adjust_transmission_queue(segments.clone(), 1.0);
        assert_eq!(adjusted.len(), 5);
        
        // Test very high speed (key frames only)
        let adjusted = controller.adjust_transmission_queue(segments.clone(), 8.0);
        assert_eq!(adjusted.len(), 2); // Only key frames
        assert!(adjusted.iter().all(|s| s.is_key_frame));
    }

    #[test]
    fn test_buffer_clearing() {
        let mut controller = DefaultPlaybackController::new();
        
        // Clear buffers (testing the method works without error)
        let result = controller.clear_buffers();
        assert!(result.is_ok());
        
        // Verify buffer health is accessible
        let buffer_health = controller.get_buffer_health();
        assert_eq!(buffer_health.video_buffer_level, 0.0);
        assert_eq!(buffer_health.audio_buffer_level, 0.0);
    }

    #[test]
    fn test_adaptive_playback_rate() {
        let mut controller = DefaultPlaybackController::new();
        
        // Test with sufficient network capacity
        let result = controller.set_adaptive_playback_rate(2.0, 5_000_000.0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2.0);
        
        // Test with limited network capacity
        let result = controller.set_adaptive_playback_rate(4.0, 2_000_000.0);
        assert!(result.is_ok());
        assert!(result.unwrap() < 4.0); // Should be reduced
        
        // Test invalid rate
        let result = controller.set_adaptive_playback_rate(-1.0, 5_000_000.0);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_transmission_timing_calculation() {
        let controller = DefaultPlaybackController::new();
        let segment = create_test_segment(0.0, 2.0, true);
        
        // Test normal speed
        let timing = controller.calculate_transmission_timing(&segment);
        assert_eq!(timing.as_secs_f64(), 2.0);
        
        // Test with different playback rate
        let mut controller = DefaultPlaybackController::new();
        let _ = controller.set_playback_rate(2.0).await;
        let timing = controller.calculate_transmission_timing(&segment);
        assert_eq!(timing.as_secs_f64(), 1.0); // Half the original duration
    }

    fn create_test_keyframe_index() -> crate::types::KeyframeIndex {
        use crate::types::{KeyframeIndex, KeyframeEntry, IndexOptimizationStrategy, FrameType};
        
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
                    timestamp: 2.0,
                    file_offset: 2048,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 4.0,
                    file_offset: 4096,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
                KeyframeEntry {
                    timestamp: 6.0,
                    file_offset: 6144,
                    frame_size: 1024,
                    gop_size: 30,
                    frame_type: FrameType::I,
                },
            ],
            total_duration: 8.0,
            index_precision: 0.1,
            memory_optimized: false,
            optimization_strategy: IndexOptimizationStrategy::Full,
            memory_usage: 256,
        }
    }

    #[test]
    fn test_find_nearest_keyframe() {
        let controller = DefaultPlaybackController::new();
        let index = create_test_keyframe_index();
        
        // Test exact keyframe positions
        let keyframe = controller.find_nearest_keyframe(2.0, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 2.0);
        
        // Test position between keyframes (should return previous keyframe)
        let keyframe = controller.find_nearest_keyframe(3.5, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 2.0);
        
        // Test position before first keyframe
        let keyframe = controller.find_nearest_keyframe(-1.0, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 0.0);
        
        // Test position after last keyframe
        let keyframe = controller.find_nearest_keyframe(10.0, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 6.0);
        
        // Test edge case: exactly at first keyframe
        let keyframe = controller.find_nearest_keyframe(0.0, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 0.0);
    }

    #[tokio::test]
    async fn test_seek_to_keyframe() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_test_keyframe_index();
        
        // Test valid seek to keyframe
        let result = controller.seek_to_keyframe(3.5, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 3.5);
        assert_eq!(seek_result.actual_time, 2.0); // Should align to previous keyframe
        assert_eq!(seek_result.keyframe_offset, 2048);
        assert!(seek_result.precision_achieved > 0.0);
        assert!(seek_result.execution_time.as_millis() >= 0);
        
        // Verify controller position was updated
        assert_eq!(controller.get_current_position(), 2.0);
        
        // Test seek beyond duration
        let result = controller.seek_to_keyframe(10.0, &index).await;
        assert!(result.is_err());
        
        // Test negative seek
        let result = controller.seek_to_keyframe(-1.0, &index).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_seek_with_alignment() {
        let mut controller = DefaultPlaybackController::new();
        let index = create_test_keyframe_index();
        
        // Test seek to exact keyframe position
        let result = controller.seek_with_alignment(2.0, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 2.0);
        assert_eq!(seek_result.actual_time, 2.0);
        assert_eq!(seek_result.precision_achieved, 1.0); // Perfect precision
        
        // Test seek to non-keyframe position (should align to previous keyframe)
        let result = controller.seek_with_alignment(3.7, &index).await;
        assert!(result.is_ok());
        
        let seek_result = result.unwrap();
        assert_eq!(seek_result.requested_time, 3.7);
        assert_eq!(seek_result.actual_time, 2.0); // Aligned to previous keyframe
        assert!(seek_result.precision_achieved < 1.0); // Not perfect precision
    }

    #[test]
    fn test_find_optimal_keyframe() {
        let controller = DefaultPlaybackController::new();
        let mut index = create_test_keyframe_index();
        
        // Test with Full optimization strategy
        index.optimization_strategy = crate::types::IndexOptimizationStrategy::Full;
        let keyframe = controller.find_optimal_keyframe(3.5, &index);
        assert!(keyframe.is_some());
        assert_eq!(keyframe.unwrap().timestamp, 2.0);
        
        // Test with Sparse optimization strategy
        index.optimization_strategy = crate::types::IndexOptimizationStrategy::Sparse;
        let keyframe = controller.find_optimal_keyframe(3.5, &index);
        assert!(keyframe.is_some());
        
        // Test with Adaptive optimization strategy (low memory usage)
        index.optimization_strategy = crate::types::IndexOptimizationStrategy::Adaptive;
        index.memory_usage = 32; // Low memory usage
        let keyframe = controller.find_optimal_keyframe(3.5, &index);
        assert!(keyframe.is_some());
        
        // Test with Hierarchical optimization strategy
        index.optimization_strategy = crate::types::IndexOptimizationStrategy::Hierarchical;
        let keyframe = controller.find_optimal_keyframe(3.5, &index);
        assert!(keyframe.is_some());
    }

    #[test]
    fn test_validate_keyframe_index() {
        let controller = DefaultPlaybackController::new();
        
        // Test valid index
        let valid_index = create_test_keyframe_index();
        let result = controller.validate_keyframe_index(&valid_index);
        assert!(result.is_ok());
        
        // Test empty index
        let mut empty_index = create_test_keyframe_index();
        empty_index.entries.clear();
        let result = controller.validate_keyframe_index(&empty_index);
        assert!(result.is_err());
        
        // Test unsorted index
        let mut unsorted_index = create_test_keyframe_index();
        unsorted_index.entries.swap(0, 1); // Make it unsorted
        let result = controller.validate_keyframe_index(&unsorted_index);
        assert!(result.is_err());
        
        // Test invalid precision
        let mut invalid_precision_index = create_test_keyframe_index();
        invalid_precision_index.index_precision = -1.0;
        let result = controller.validate_keyframe_index(&invalid_precision_index);
        assert!(result.is_err());
        
        // Test inconsistent duration
        let mut invalid_duration_index = create_test_keyframe_index();
        invalid_duration_index.total_duration = 2.0; // Less than last keyframe timestamp
        let result = controller.validate_keyframe_index(&invalid_duration_index);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_seek_info() {
        let mut controller = DefaultPlaybackController::new();
        
        // Initially no seek info
        assert!(controller.get_seek_info().is_none());
        
        // After seek, should have seek info
        let _ = tokio_test::block_on(controller.seek(5.0));
        let seek_info = controller.get_seek_info();
        assert!(seek_info.is_some());
        
        let (last_seek, current_pos) = seek_info.unwrap();
        assert_eq!(last_seek, 5.0);
        assert_eq!(current_pos, 5.0);
    }
}