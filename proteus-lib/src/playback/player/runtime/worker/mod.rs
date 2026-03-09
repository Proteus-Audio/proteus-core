//! Playback worker internals.
//!
//! This module is split so responsibilities are explicit:
//! - [`context`] defines captured shared thread state.
//! - [`guard`] tracks playback-thread liveness.
//! - [`runner`] executes the long-running receive/append/drain loop.

mod context;
mod guard;
mod runner;

pub(in crate::playback::player::runtime) use context::ThreadContext;
pub(in crate::playback::player::runtime) use runner::{
    open_output_stream_with_retry, run_playback_thread,
};

#[cfg(test)]
mod tests {
    use super::ThreadContext;

    #[test]
    fn thread_context_type_is_exported_within_runtime() {
        let name = std::any::type_name::<ThreadContext>();
        assert!(name.contains("ThreadContext"));
    }
}
