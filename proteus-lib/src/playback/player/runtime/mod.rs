//! Internal runtime plumbing for the `Player` execution thread.
//!
//! The runtime is intentionally split so construction-time concerns stay
//! separate from the long-lived audio worker loop:
//! - [`thread`] handles thread bootstrap and shared state capture.
//! - [`worker`] runs the real-time receive/append/drain loop.

mod thread;
mod worker;

/// Return current wall-clock time in milliseconds since Unix epoch.
///
/// The runtime uses this as a lightweight monotonic-enough marker for
/// diagnostics and debug visibility (`last_chunk_ms`, `last_time_update_ms`).
pub(super) fn now_ms() -> u64 {
    use std::time::SystemTime;

    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
