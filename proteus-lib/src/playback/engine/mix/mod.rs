//! Mixing-thread orchestration for playback.
//!
//! This module exposes the public API used by `PlayerEngine` and delegates
//! implementation details to focused submodules:
//! - `types`: argument and transition structs.
//! - `effects`: effect-chain processing helpers.
//! - `debug`: debug-only naming helpers.
//! - `runner`: thin public entrypoint wrapper.
//! - `loop_core`: long-running mix loop implementation.

mod buffer_mixer;
mod debug;
mod decoder_events;
mod effects;
mod output_stage;
mod runner;
mod track_stage;
mod types;
mod utils;

pub use runner::spawn_mix_thread;
pub use types::MixThreadArgs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_module_exports_public_api() {
        let _ = spawn_mix_thread;
        let _ = core::mem::size_of::<MixThreadArgs>();
    }
}
