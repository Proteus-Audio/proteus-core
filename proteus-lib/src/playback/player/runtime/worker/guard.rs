//! Liveness guard for playback worker threads.
//!
//! # Ordering contract: `playback_thread_exists`
//!
//! This flag signals thread liveness from the worker to external observers:
//! - The spawner sets `true` with `Release` before `thread::spawn`.
//! - The guard clears `false` with `Release` when the worker exits.
//! - All external loads use `Acquire` to synchronize-with those stores.
//!
//! The redundant `true` store in `new` is `Relaxed` because the spawner has
//! already published the flag with `Release` and the spawn itself establishes
//! happens-before; no additional ordering is needed from within the thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Guard that keeps `playback_thread_exists` in sync with worker lifetime.
pub(super) struct PlaybackThreadGuard {
    exists: Arc<AtomicBool>,
}

impl PlaybackThreadGuard {
    /// Mark the thread as alive.
    pub(super) fn new(exists: Arc<AtomicBool>) -> Self {
        // Redundant confirm from inside the thread; spawner already set this
        // with Release before thread::spawn, so Relaxed is sufficient here.
        exists.store(true, Ordering::Relaxed);
        Self { exists }
    }
}

impl Drop for PlaybackThreadGuard {
    fn drop(&mut self) {
        // Release: publish thread-exit to any Acquire load of this flag.
        self.exists.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::PlaybackThreadGuard;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn guard_sets_alive_on_create_and_clears_on_drop() {
        let exists = Arc::new(AtomicBool::new(false));
        {
            let _guard = PlaybackThreadGuard::new(exists.clone());
            assert!(exists.load(Ordering::Acquire));
        }
        assert!(!exists.load(Ordering::Acquire));
    }

    #[test]
    fn guard_drop_publishes_thread_done_visible_across_threads() {
        let exists = Arc::new(AtomicBool::new(false));
        let writer = exists.clone();
        let handle = std::thread::spawn(move || {
            let _guard = PlaybackThreadGuard::new(writer);
            // guard drops here — stores false with Release
        });
        handle.join().unwrap();
        // Acquire load sees the Release store from the spawned thread.
        assert!(!exists.load(Ordering::Acquire));
    }
}
