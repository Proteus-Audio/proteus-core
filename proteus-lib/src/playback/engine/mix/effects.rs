//! DSP effect-chain helpers for the mix runtime.

use crate::dsp::effects::{AudioEffect, EffectContext};

/// Run the current effect chain over a chunk of audio using pre-allocated scratch buffers.
///
/// Processes `input` through each effect in order, ping-ponging between `scratch_a`
/// and `scratch_b` to eliminate per-stage heap allocations. After the call, the
/// processed output resides in `scratch_a`; `scratch_b` is a scratch residual.
///
/// Both buffers are cleared and resized as needed. Reserve sufficient capacity in
/// them once (sized to the expected chunk length) to achieve zero-allocation
/// steady-state processing.
///
/// # Arguments
///
/// * `effects` - Mutable ordered list of effects.
/// * `input` - Interleaved PCM input samples.
/// * `context` - Runtime effect context (sample rate/channels/container).
/// * `drain` - Whether effects should emit any remaining tail state.
/// * `scratch_a` - First scratch buffer (holds result after the call).
/// * `scratch_b` - Second scratch buffer (used internally for ping-pong).
pub(super) fn run_effect_chain(
    effects: &mut [AudioEffect],
    input: &[f32],
    context: &EffectContext,
    drain: bool,
    scratch_a: &mut Vec<f32>,
    scratch_b: &mut Vec<f32>,
) {
    scratch_a.clear();
    scratch_a.extend_from_slice(input);

    for effect in effects.iter_mut() {
        scratch_b.clear();
        effect.process_into(scratch_a, scratch_b, context, drain);
        std::mem::swap(scratch_a, scratch_b);
    }
    // scratch_a holds the final processed output.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> EffectContext {
        EffectContext {
            sample_rate: 48_000,
            channels: 2,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        }
    }

    #[test]
    fn run_effect_chain_passthrough_when_empty_effects() {
        let mut effects = Vec::new();
        let input = vec![0.25_f32, -0.25];
        let mut scratch_a = Vec::new();
        let mut scratch_b = Vec::new();
        run_effect_chain(&mut effects, &input, &context(), false, &mut scratch_a, &mut scratch_b);
        assert_eq!(scratch_a, input);
    }

    #[test]
    fn run_effect_chain_result_in_scratch_a_after_single_effect() {
        use crate::dsp::effects::{AudioEffect, GainEffect};
        let mut effect = GainEffect::default();
        effect.enabled = true;
        effect.settings.gain = 2.0;
        let mut effects = vec![AudioEffect::Gain(effect)];
        let input = vec![0.5_f32, -0.5];
        let mut scratch_a = Vec::new();
        let mut scratch_b = Vec::new();
        run_effect_chain(&mut effects, &input, &context(), false, &mut scratch_a, &mut scratch_b);
        assert_eq!(scratch_a, vec![1.0_f32, -1.0]);
    }
}
