use std::panic::{self, AssertUnwindSafe};

use crate::container::prot::ShuffleSource;

use super::{
    reserve_source_room, source_room_status, DecodeBackpressure, DecodeBackpressureInstance,
    DecodeBackpressureState, SourceKey,
};

fn state_with_one_source(buffered: usize, capacity: usize) -> DecodeBackpressureState {
    let mut state = DecodeBackpressureState::default();
    state.instances.push(DecodeBackpressureInstance {
        capacity_samples: capacity,
        buffered_samples: buffered,
        reserved_samples: 0,
        finished: false,
    });
    state
        .source_to_instances
        .insert(SourceKey::from(&ShuffleSource::TrackId(1)), vec![0]);
    state
}

#[test]
fn source_room_status_disallows_when_target_not_available() {
    let state = state_with_one_source(9, 10);
    let status = source_room_status(&state, &SourceKey::TrackId(1), 4, true);
    assert!(!status.allowed);
}

#[test]
fn reserve_source_room_clamps_to_capacity() {
    let mut state = state_with_one_source(0, 8);
    reserve_source_room(&mut state, &SourceKey::TrackId(1), 32);
    assert_eq!(state.instances[0].reserved_samples, 8);
}

#[test]
fn has_waiters_recovers_after_state_poison() {
    let backpressure = DecodeBackpressure::default();
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let _guard = backpressure.lock_state_recoverable();
        panic!("poison decode backpressure state");
    }));

    assert!(!backpressure.has_waiters());
}
