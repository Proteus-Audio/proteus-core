//! Legacy track buffering modules.
//!
//! These modules predate the current `playback::engine` architecture. They are
//! retained so their tests continue to compile and run, but the active playback
//! path uses `playback::engine::mix::runner::decode` instead.

#![allow(dead_code)]

pub mod buffer;
pub mod container;
pub(crate) mod context;
pub mod single;
