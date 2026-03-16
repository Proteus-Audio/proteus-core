use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

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

fn backpressure_full(capacity: usize) -> DecodeBackpressure {
    let state = state_with_one_source(capacity, capacity);
    DecodeBackpressure {
        state: std::sync::Mutex::new(state),
        cv: std::sync::Condvar::new(),
    }
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
fn wait_exits_on_shutdown_without_producer_notify() {
    let bp = Arc::new(backpressure_full(1024));
    let abort = Arc::new(AtomicBool::new(false));

    let bp_clone = Arc::clone(&bp);
    let abort_clone = Arc::clone(&abort);
    let source = SourceKey::TrackId(1);

    let handle =
        std::thread::spawn(move || bp_clone.wait_for_source_room(&source, 512, &abort_clone));

    // Give the waiter time to enter the wait loop (at least one timeout cycle).
    std::thread::sleep(Duration::from_millis(120));

    // Signal shutdown without any producer notification.
    bp.shutdown();

    let result = handle.join().expect("waiter thread panicked");
    assert!(!result, "waiter should return false on shutdown");
}

#[test]
fn wait_exits_on_abort_without_producer_notify() {
    let bp = Arc::new(backpressure_full(1024));
    let abort = Arc::new(AtomicBool::new(false));

    let bp_clone = Arc::clone(&bp);
    let abort_clone = Arc::clone(&abort);
    let source = SourceKey::TrackId(1);

    let handle =
        std::thread::spawn(move || bp_clone.wait_for_source_room(&source, 512, &abort_clone));

    // Give the waiter time to enter the wait loop.
    std::thread::sleep(Duration::from_millis(120));

    // Set abort without any producer notification or condvar wake.
    abort.store(true, std::sync::atomic::Ordering::Relaxed);

    let result = handle.join().expect("waiter thread panicked");
    assert!(!result, "waiter should return false on abort");
}

#[test]
fn wait_exits_when_notifier_thread_exits_and_shutdown_set() {
    let bp = Arc::new(backpressure_full(1024));
    let abort = Arc::new(AtomicBool::new(false));

    let bp_clone = Arc::clone(&bp);
    let abort_clone = Arc::clone(&abort);
    let source = SourceKey::TrackId(1);

    let handle =
        std::thread::spawn(move || bp_clone.wait_for_source_room(&source, 512, &abort_clone));

    // Simulate a notifier thread that exits without ever calling notify.
    let notifier = std::thread::spawn(|| {
        // Intentionally do nothing — no notify, no room freed.
    });
    notifier.join().expect("notifier thread panicked");

    // Even though the notifier exited without waking anyone, the waiter
    // should still exit once shutdown is set, thanks to the timeout.
    std::thread::sleep(Duration::from_millis(120));
    bp.shutdown();

    let result = handle.join().expect("waiter thread panicked");
    assert!(
        !result,
        "waiter should return false after notifier exited and shutdown set"
    );
}
