use std::collections::HashMap;

use crate::container::prot::{
    ActiveWindow, RuntimeInstanceMeta, RuntimeInstancePlan, ShuffleSource,
};

use super::{BufferMixer, FillState, SourceKey};

/// Build a small two-track runtime plan used by buffer mixer unit tests.
fn simple_plan() -> RuntimeInstancePlan {
    RuntimeInstancePlan {
        logical_track_count: 2,
        instances: vec![
            RuntimeInstanceMeta {
                instance_id: 0,
                logical_track_index: 0,
                slot_index: 0,
                source_key: ShuffleSource::TrackId(1),
                active_windows: vec![ActiveWindow {
                    start_ms: 0,
                    end_ms: Some(1000),
                }],
                selection_index: 0,
                occurrence_index: 0,
            },
            RuntimeInstanceMeta {
                instance_id: 1,
                logical_track_index: 1,
                slot_index: 1,
                source_key: ShuffleSource::TrackId(2),
                active_windows: vec![ActiveWindow {
                    start_ms: 0,
                    end_ms: Some(1000),
                }],
                selection_index: 0,
                occurrence_index: 0,
            },
        ],
        event_boundaries_ms: vec![0],
    }
}

#[test]
/// Verifies packet routing writes samples only to matching source instances.
fn route_packet_targets_and_zero_fills_instances() {
    let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 8, HashMap::new(), 4);

    let decision = mixer.route_packet(&[1.0, 1.0, 0.5, 0.5], SourceKey::TrackId(1), 0.0);
    assert_eq!(decision.sample_targets_written, vec![0]);
    assert!(decision.zero_fill_targets_written.is_empty());
    assert!(!decision.ignored);
}

#[test]
/// Verifies mix readiness and sample consumption stay in lockstep.
fn readiness_and_take_samples_are_synchronized() {
    let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 16, HashMap::new(), 4);

    mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
    assert!(!mixer.mix_ready());
    assert!(mixer.take_samples().is_none());

    mixer.route_packet(&[0.5, 0.5, 0.5, 0.5], SourceKey::TrackId(2), 0.0);
    assert!(mixer.mix_ready());

    let mixed = mixer.take_samples().expect("mixed samples");
    assert_eq!(mixed.len(), 4);
    assert_eq!(mixed, vec![0.75, 0.75, 0.75, 0.75]);
}

#[test]
/// Verifies finish signals propagate to per-track and global finished state.
fn signal_finish_propagates_track_and_mix_finished() {
    let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 8, HashMap::new(), 4);
    mixer.signal_finish(&SourceKey::TrackId(1));
    assert!(mixer.track_finished(0));
    assert!(!mixer.mix_finished());

    mixer.signal_finish(&SourceKey::TrackId(2));
    assert!(mixer.track_finished(1));
    assert!(mixer.mix_finished());
}

#[test]
/// Verifies aggregate fill-state reporting reflects per-instance fullness.
fn fill_state_aggregates_as_expected() {
    let mut track_mix = HashMap::new();
    track_mix.insert(0usize, (1.0_f32, 0.0_f32));
    let mut mixer = BufferMixer::new(simple_plan(), 48_000, 2, 2, track_mix, 4);
    assert_eq!(mixer.mix_fill_state(), FillState::NotFull);

    let _ = mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
    assert!(matches!(
        mixer.mix_fill_state(),
        FillState::Partial | FillState::Full
    ));
}

#[test]
/// Verifies packets before a window start are represented as aligned zero-fill.
fn route_packet_zero_fills_when_packet_is_before_window_start() {
    let plan = RuntimeInstancePlan {
        logical_track_count: 1,
        instances: vec![RuntimeInstanceMeta {
            instance_id: 0,
            logical_track_index: 0,
            slot_index: 0,
            source_key: ShuffleSource::TrackId(1),
            active_windows: vec![ActiveWindow {
                start_ms: 1000,
                end_ms: Some(2000),
            }],
            selection_index: 0,
            occurrence_index: 0,
        }],
        event_boundaries_ms: vec![0, 1000],
    };
    let mut mixer = BufferMixer::new(plan, 48_000, 2, 16, HashMap::new(), 4);

    let decision = mixer.route_packet(&[1.0, 1.0, 1.0, 1.0], SourceKey::TrackId(1), 0.0);
    assert!(decision.sample_targets_written.is_empty());
    assert_eq!(decision.zero_fill_targets_written, vec![0]);
    assert!(mixer.mix_ready());

    let mixed = mixer.take_samples().expect("zero-filled samples");
    assert_eq!(mixed, vec![0.0, 0.0, 0.0, 0.0]);
}
