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
pub(in crate::playback::player::runtime) use runner::run_playback_thread;
