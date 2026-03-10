//! Playback worker internals.
//!
//! This module is split so responsibilities are explicit:
//! - [`context`] defines captured shared thread state.
//! - [`guard`] tracks playback-thread liveness.
//! - [`runner`] executes the long-running receive loop entry points.
//! - [`sink`] manages output stream and sink appends.
//! - [`transitions`] applies transport-state changes.
//! - [`timing`] maintains playback time and drain completion.

mod context;
mod guard;
mod runner;
mod sink;
mod timing;
mod transitions;

pub(in crate::playback::player::runtime) use context::ThreadContext;
pub(in crate::playback::player::runtime) use runner::run_playback_thread;
pub(in crate::playback::player::runtime) use sink::open_output_stream_with_retry;

#[cfg(test)]
mod tests {
    use super::ThreadContext;

    #[test]
    fn thread_context_type_is_exported_within_runtime() {
        let name = std::any::type_name::<ThreadContext>();
        assert!(name.contains("ThreadContext"));
    }
}
