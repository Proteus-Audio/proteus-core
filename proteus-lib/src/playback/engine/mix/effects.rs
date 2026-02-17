//! DSP effect-chain helpers for the mix runtime.

use crate::dsp::effects::{AudioEffect, EffectContext};

/// Run the current effect chain over a chunk of audio.
///
/// # Arguments
///
/// * `effects` - Mutable ordered list of effects.
/// * `input` - Interleaved PCM input samples.
/// * `context` - Runtime effect context (sample rate/channels/container).
/// * `drain` - Whether effects should emit any remaining tail state.
///
/// # Returns
///
/// Processed interleaved samples.
pub(super) fn run_effect_chain(
    effects: &mut [AudioEffect],
    input: &[f32],
    context: &EffectContext,
    drain: bool,
) -> Vec<f32> {
    let mut current = input.to_vec();
    for effect in effects.iter_mut() {
        current = effect.process(&current, context, drain);
    }
    current
}
