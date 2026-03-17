//! Shared mutex poison-policy helpers for playback runtime state.

use std::sync::{Condvar, LockResult, Mutex, MutexGuard, WaitTimeoutResult};
use std::time::Duration;

use log::warn;

fn recover_poison<'a, T>(
    result: LockResult<MutexGuard<'a, T>>,
    label: &str,
    rationale: &str,
) -> MutexGuard<'a, T> {
    match result {
        Ok(guard) => guard,
        Err(err) => {
            warn!(
                "{label} lock poisoned; recovering with the inner value because {rationale}"
            );
            err.into_inner()
        }
    }
}

pub(crate) fn lock_invariant<'a, T>(
    mutex: &'a Mutex<T>,
    label: &str,
    rationale: &str,
) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|_| {
        panic!(
            "{label} lock poisoned — invariant-only state cannot recover because {rationale}"
        )
    })
}

pub(crate) fn lock_recoverable<'a, T>(
    mutex: &'a Mutex<T>,
    label: &str,
    rationale: &str,
) -> MutexGuard<'a, T> {
    recover_poison(mutex.lock(), label, rationale)
}

pub(crate) fn wait_recoverable<'a, T>(
    condvar: &Condvar,
    guard: MutexGuard<'a, T>,
    label: &str,
    rationale: &str,
) -> MutexGuard<'a, T> {
    recover_poison(condvar.wait(guard), label, rationale)
}

pub(crate) fn wait_timeout_recoverable<'a, T>(
    condvar: &Condvar,
    guard: MutexGuard<'a, T>,
    timeout: Duration,
    label: &str,
    rationale: &str,
) -> (MutexGuard<'a, T>, WaitTimeoutResult) {
    match condvar.wait_timeout(guard, timeout) {
        Ok(result) => result,
        Err(err) => {
            warn!(
                "{label} lock poisoned; recovering with the inner value because {rationale}"
            );
            err.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Condvar, Mutex};

    use super::{lock_invariant, lock_recoverable, wait_recoverable};

    #[test]
    fn lock_recoverable_returns_inner_after_poison() {
        let mutex = Arc::new(Mutex::new(41_u32));
        let clone = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock().expect("test mutex should lock");
            panic!("poison recoverable mutex");
        })
        .join();

        let mut guard = lock_recoverable(&mutex, "test recoverable", "the value is disposable");
        *guard += 1;
        assert_eq!(*guard, 42);
    }

    #[test]
    fn wait_recoverable_returns_inner_after_poison() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let clone = Arc::clone(&pair);
        let _ = std::thread::spawn(move || {
            let (lock, _cv) = &*clone;
            let _guard = lock.lock().expect("test mutex should lock");
            panic!("poison recoverable condvar mutex");
        })
        .join();

        let (lock, cv) = &*pair;
        let guard = lock_recoverable(lock, "test condvar", "the flag is disposable");
        let pair_for_notify = Arc::clone(&pair);
        let notifier = std::thread::spawn(move || {
            let (lock, cv) = &*pair_for_notify;
            let mut guard = lock_recoverable(lock, "test condvar", "the flag is disposable");
            *guard = true;
            cv.notify_one();
        });
        let guard = wait_recoverable(cv, guard, "test condvar", "the flag is disposable");
        notifier.join().expect("notifier thread should complete");
        assert!(*guard);
    }

    #[test]
    #[should_panic(expected = "invariant-only state cannot recover")]
    fn lock_invariant_still_panics_on_poison() {
        let mutex = Arc::new(Mutex::new(0_u32));
        let clone = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock().expect("test mutex should lock");
            panic!("poison invariant mutex");
        })
        .join();

        let _guard = lock_invariant(&mutex, "test invariant", "consistency is required");
    }
}
