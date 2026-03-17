//! Mixing-thread orchestration for playback.
//!
//! This module exposes the public API used by `PlayerEngine` and delegates
//! implementation details to focused submodules:
//! - `types`: argument and transition structs.
//! - `effects`: effect-chain processing helpers.
//! - `debug`: debug-only naming helpers.
//! - `runner`: long-running mix loop and public entrypoint wrapper.
//! - `track_stage` / `output_stage`: staged helpers used by the runner.

mod buffer_mixer;
mod cover_map;
mod debug;
mod decoder_events;
mod effects;
mod output_stage;
mod runner;
mod track_stage;
mod types;

pub use runner::spawn_mix_thread;
pub use types::{EffectSettingsCommand, MixThreadArgs};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_module_exports_public_api() {
        let _ = spawn_mix_thread;
        let _ = core::mem::size_of::<MixThreadArgs>();
    }
}
