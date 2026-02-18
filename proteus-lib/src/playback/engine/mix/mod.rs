//! Mixing-thread orchestration for playback.
//!
//! This module exposes the public API used by `PlayerEngine` and delegates
//! implementation details to focused submodules:
//! - `types`: argument and transition structs.
//! - `effects`: effect-chain processing helpers.
//! - `debug`: debug-only naming helpers.
//! - `runner`: thin public entrypoint wrapper.
//! - `loop_core`: long-running mix loop implementation.

mod debug;
mod effects;
mod output_stage;
mod runner;
mod source_spawner;
mod track_mix;
mod types;

pub use runner::spawn_mix_thread;
pub use types::MixThreadArgs;
