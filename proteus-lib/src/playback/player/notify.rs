//! Event-driven notification primitive for the playback worker.
//!
//! [`WorkerNotify`] replaces fixed-interval polling and sleep-based wakeups
//! with a condvar that is signalled by control operations (pause, resume,
//! abort) so the worker thread can react immediately.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Condvar-based notification used to wake the playback worker from
/// backpressure waits when transport state changes or an abort is requested.
///
/// The internal `pending` flag prevents lost wakeups when `notify` fires
/// before the worker enters its wait.
pub(in crate::playback::player) struct WorkerNotify {
    pending: Mutex<bool>,
    condvar: Condvar,
}

impl WorkerNotify {
    /// Create a new idle notification.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /// Signal the worker to wake up.
    pub fn notify(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        *pending = true;
        self.condvar.notify_one();
    }

    /// Block until notified or until `timeout` elapses.
    pub fn wait_timeout(&self, timeout: Duration) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if !*pending {
            let (guard, _) = self
                .condvar
                .wait_timeout(pending, timeout)
                .unwrap_or_else(|e| e.into_inner());
            pending = guard;
        }
        *pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerNotify;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn notify_before_wait_returns_immediately() {
        let n = WorkerNotify::new();
        n.notify();
        let start = Instant::now();
        n.wait_timeout(Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn wait_timeout_returns_after_timeout_without_notify() {
        let n = WorkerNotify::new();
        let start = Instant::now();
        n.wait_timeout(Duration::from_millis(20));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(15));
        assert!(elapsed < Duration::from_millis(200));
    }

    #[test]
    fn notify_from_another_thread_wakes_waiter() {
        let n = Arc::new(WorkerNotify::new());
        let n2 = n.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            n2.notify();
        });
        let start = Instant::now();
        n.wait_timeout(Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_millis(500));
        handle.join().unwrap();
    }
}
