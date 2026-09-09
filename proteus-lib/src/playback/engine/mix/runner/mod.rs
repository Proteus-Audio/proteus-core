//! Core mix-thread runtime loop implementation.

mod decode;
mod effect_metering;
mod effects_runtime;
mod loop_body;
mod startup;
mod state;

use rodio::buffer::SamplesBuffer;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use super::MixThreadArgs;

/// Spawn the mixing thread and return a receiver of mixed audio buffers.
pub fn spawn_mix_thread(
    args: MixThreadArgs,
) -> (mpsc::Receiver<(SamplesBuffer, f64)>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer, f64)>(1);
    let handle = thread::spawn(move || {
        let startup_trace = Instant::now();
        let Some(mut state) = startup::setup_mix_state(args, sender, startup_trace) else {
            return;
        };
        loop_body::run_mix_loop(&mut state, startup_trace);
        loop_body::teardown_mix(state);
    });
    (receiver, handle)
}

#[cfg(test)]
mod tests {
    use super::loop_body::{
        DRAIN_SILENCE_EPSILON, DRAIN_SILENT_PASSES_TO_STOP, MAX_EFFECT_DRAIN_PASSES,
    };

    #[test]
    fn drain_constants_are_positive() {
        assert!(MAX_EFFECT_DRAIN_PASSES > 0);
        assert!(DRAIN_SILENT_PASSES_TO_STOP > 0);
        assert!(DRAIN_SILENCE_EPSILON > 0.0);
    }
}
