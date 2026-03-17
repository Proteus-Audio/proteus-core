//! Event-driven notification primitive for the playback worker.
//!
//! [`WorkerNotify`] replaces fixed-interval polling and sleep-based wakeups
//! with a condvar that is signalled by control operations (pause, resume,
//! abort) so the worker thread can react immediately.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::playback::mutex_policy::{lock_recoverable, wait_timeout_recoverable};

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
        let mut pending = self.lock_pending_recoverable();
        *pending = true;
        self.condvar.notify_one();
    }

    /// Block until notified or until `timeout` elapses.
    pub fn wait_timeout(&self, timeout: Duration) {
        let mut pending = self.lock_pending_recoverable();
        if !*pending {
            let (guard, _) = self.wait_pending_recoverable(pending, timeout);
            pending = guard;
        }
        *pending = false;
    }

    /// Recoverable poison policy: pending worker notifications are disposable coordination state.
    fn lock_pending_recoverable(&self) -> std::sync::MutexGuard<'_, bool> {
        lock_recoverable(
            &self.pending,
            "worker notify pending flag",
            "worker wake notifications are disposable coordination state",
        )
    }

    /// Recoverable poison policy: pending worker notifications are disposable coordination state.
    fn wait_pending_recoverable<'a>(
        &self,
        guard: std::sync::MutexGuard<'a, bool>,
        timeout: Duration,
    ) -> (std::sync::MutexGuard<'a, bool>, std::sync::WaitTimeoutResult) {
        wait_timeout_recoverable(
            &self.condvar,
            guard,
            timeout,
            "worker notify pending flag",
            "worker wake notifications are disposable coordination state",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerNotify;
    use std::sync::Arc;
    use std::thread;
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
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            n2.notify();
        });
        let start = Instant::now();
        n.wait_timeout(Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_millis(500));
        handle.join().unwrap();
    }

    #[test]
    fn notify_recovers_after_pending_mutex_poison() {
        let n = Arc::new(WorkerNotify::new());
        let poisoned = n.clone();
        let _ = thread::spawn(move || {
            let _guard = poisoned.pending.lock().expect("test mutex should lock");
            panic!("poison worker notify");
        })
        .join();

        n.notify();
        n.wait_timeout(Duration::from_millis(1));
        assert!(!*n.lock_pending_recoverable());
    }
}
