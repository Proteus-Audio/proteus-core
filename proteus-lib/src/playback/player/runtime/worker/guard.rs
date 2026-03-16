//! Liveness guard for playback worker threads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Guard that keeps `playback_thread_exists` in sync with worker lifetime.
pub(super) struct PlaybackThreadGuard {
    exists: Arc<AtomicBool>,
}

impl PlaybackThreadGuard {
    /// Mark the thread as alive.
    pub(super) fn new(exists: Arc<AtomicBool>) -> Self {
        exists.store(true, Ordering::Relaxed);
        Self { exists }
    }
}

impl Drop for PlaybackThreadGuard {
    fn drop(&mut self) {
        self.exists.store(false, Ordering::Relaxed);
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
            assert!(exists.load(Ordering::Relaxed));
        }
        assert!(!exists.load(Ordering::Relaxed));
    }
}
