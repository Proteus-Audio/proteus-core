//! DSP effect-chain helpers for the mix runtime.

use crate::dsp::effects::{AudioEffect, EffectContext};

pub(super) trait EffectChainObserver {
    fn before_effect(
        &mut self,
        effect_index: usize,
        effect: &AudioEffect,
        input: &[f32],
        channels: usize,
    );

    fn after_effect(
        &mut self,
        effect_index: usize,
        effect: &AudioEffect,
        output: &[f32],
        channels: usize,
    );
}

#[derive(Clone, Debug)]
pub(super) struct EffectEnableFade {
    current_mix: f32,
    target_mix: f32,
    increment: f32,
    remaining_frames: usize,
    target_enabled: bool,
}

impl EffectEnableFade {
    pub(super) fn new(current_mix: f32, target_enabled: bool, ramp_frames: usize) -> Self {
        let target_mix = if target_enabled { 1.0 } else { 0.0 };
        if ramp_frames == 0 || (current_mix - target_mix).abs() < f32::EPSILON {
            return Self {
                current_mix: target_mix,
                target_mix,
                increment: 0.0,
                remaining_frames: 0,
                target_enabled,
            };
        }

        Self {
            current_mix,
            target_mix,
            increment: (target_mix - current_mix) / ramp_frames as f32,
            remaining_frames: ramp_frames,
            target_enabled,
        }
    }

    pub(super) fn next_mix(&mut self) -> f32 {
        if self.remaining_frames == 0 {
            return self.current_mix;
        }
        self.remaining_frames -= 1;
        if self.remaining_frames == 0 {
            self.current_mix = self.target_mix;
        } else {
            self.current_mix += self.increment;
        }
        self.current_mix
    }

    pub(super) fn current_mix(&self) -> f32 {
        self.current_mix
    }

    pub(super) fn target_enabled(&self) -> bool {
        self.target_enabled
    }

    pub(super) fn is_complete(&self) -> bool {
        self.remaining_frames == 0
    }
}

/// Inputs and scratch state for one effect-chain run.
pub(super) struct EffectChainRun<'a> {
    pub(super) effects: &'a mut [AudioEffect],
    pub(super) input: &'a [f32],
    pub(super) context: &'a EffectContext,
    pub(super) drain: bool,
    pub(super) scratch_a: &'a mut Vec<f32>,
    pub(super) scratch_b: &'a mut Vec<f32>,
    pub(super) observer: Option<&'a mut dyn EffectChainObserver>,
    pub(super) enable_fades: Option<&'a mut [Option<EffectEnableFade>]>,
}

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
/// The processed output resides in `run.scratch_a` after this call.
pub(super) fn run_effect_chain(run: EffectChainRun<'_>) {
    let EffectChainRun {
        effects,
        input,
        context,
        drain,
        scratch_a,
        scratch_b,
        observer,
        enable_fades,
    } = run;
    scratch_a.clear();
    scratch_a.extend_from_slice(input);

    let channels = context.channels().max(1);
    let mut observer = observer;
    let mut enable_fades = enable_fades;
    for (index, effect) in effects.iter_mut().enumerate() {
        if let Some(observer) = observer.as_deref_mut() {
            observer.before_effect(index, effect, scratch_a, channels);
        }
        if let Some(fades) = enable_fades.as_deref_mut() {
            if let Some(fade) = fades.get_mut(index).and_then(Option::as_mut) {
                scratch_b.clear();
                effect.process_into(scratch_a, scratch_b, context, drain);
                crossfade_enabled_output(scratch_a, scratch_b, fade, channels);
                if let Some(observer) = observer.as_deref_mut() {
                    observer.after_effect(index, effect, scratch_a, channels);
                }

                if fade.is_complete() {
                    if !fade.target_enabled() {
                        effect.reset_state();
                    }
                    set_audio_effect_enabled(effect, fade.target_enabled());
                    fades[index] = None;
                }
                continue;
            }
        }

        scratch_b.clear();
        effect.process_into(scratch_a, scratch_b, context, drain);
        std::mem::swap(scratch_a, scratch_b);
        if let Some(observer) = observer.as_deref_mut() {
            observer.after_effect(index, effect, scratch_a, channels);
        }
    }
    // scratch_a holds the final processed output.
}

fn crossfade_enabled_output(
    dry: &mut Vec<f32>,
    wet: &[f32],
    fade: &mut EffectEnableFade,
    channels: usize,
) {
    let original_dry_len = dry.len();
    let total_len = original_dry_len.max(wet.len());
    dry.resize(total_len, 0.0);

    for frame_start in (0..total_len).step_by(channels) {
        let wet_mix = fade.next_mix();
        let dry_mix = 1.0 - wet_mix;
        let frame_end = (frame_start + channels).min(total_len);
        for (sample_index, dry_sample) in
            dry.iter_mut().enumerate().take(frame_end).skip(frame_start)
        {
            let dry_value = if sample_index < original_dry_len {
                *dry_sample
            } else {
                0.0
            };
            let wet_sample = wet.get(sample_index).copied().unwrap_or(0.0);
            *dry_sample = (dry_value * dry_mix) + (wet_sample * wet_mix);
        }
    }
}

fn set_audio_effect_enabled(effect: &mut AudioEffect, enabled: bool) {
    match effect {
        AudioEffect::Gain(effect) => effect.enabled = enabled,
        AudioEffect::Pan(effect) => effect.enabled = enabled,
        AudioEffect::Distortion(effect) => effect.enabled = enabled,
        AudioEffect::DelayReverb(effect) => effect.enabled = enabled,
        AudioEffect::DiffusionReverb(effect) => effect.enabled = enabled,
        AudioEffect::ConvolutionReverb(effect) => effect.enabled = enabled,
        AudioEffect::LowPassFilter(effect) => effect.enabled = enabled,
        AudioEffect::HighPassFilter(effect) => effect.enabled = enabled,
        AudioEffect::Compressor(effect) => effect.enabled = enabled,
        AudioEffect::Limiter(effect) => effect.enabled = enabled,
        AudioEffect::MultibandEq(effect) => effect.enabled = enabled,
    }
}

pub(super) fn audio_effect_enabled(effect: &AudioEffect) -> bool {
    match effect {
        AudioEffect::Gain(effect) => effect.enabled,
        AudioEffect::Pan(effect) => effect.enabled,
        AudioEffect::Distortion(effect) => effect.enabled,
        AudioEffect::DelayReverb(effect) => effect.enabled,
        AudioEffect::DiffusionReverb(effect) => effect.enabled,
        AudioEffect::ConvolutionReverb(effect) => effect.enabled,
        AudioEffect::LowPassFilter(effect) => effect.enabled,
        AudioEffect::HighPassFilter(effect) => effect.enabled,
        AudioEffect::Compressor(effect) => effect.enabled,
        AudioEffect::Limiter(effect) => effect.enabled,
        AudioEffect::MultibandEq(effect) => effect.enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> EffectContext {
        EffectContext::new(48_000, 2, None, None, -60.0).unwrap()
    }

    #[test]
    fn run_effect_chain_passthrough_when_empty_effects() {
        let mut effects = Vec::new();
        let input = vec![0.25_f32, -0.25];
        let mut scratch_a = Vec::new();
        let mut scratch_b = Vec::new();
        run_effect_chain(EffectChainRun {
            effects: &mut effects,
            input: &input,
            context: &context(),
            drain: false,
            scratch_a: &mut scratch_a,
            scratch_b: &mut scratch_b,
            observer: None,
            enable_fades: None,
        });
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
        run_effect_chain(EffectChainRun {
            effects: &mut effects,
            input: &input,
            context: &context(),
            drain: false,
            scratch_a: &mut scratch_a,
            scratch_b: &mut scratch_b,
            observer: None,
            enable_fades: None,
        });
        assert_eq!(scratch_a, vec![1.0_f32, -1.0]);
    }

    #[test]
    fn run_effect_chain_passthrough_when_empty() {
        let mut effects = Vec::new();
        let input = vec![0.25_f32, -0.25];
        let context = EffectContext::new(48_000, 2, None, None, -60.0).unwrap();
        let mut scratch_a = Vec::new();
        let mut scratch_b = Vec::new();
        run_effect_chain(EffectChainRun {
            effects: &mut effects,
            input: &input,
            context: &context,
            drain: false,
            scratch_a: &mut scratch_a,
            scratch_b: &mut scratch_b,
            observer: None,
            enable_fades: None,
        });
        assert_eq!(scratch_a, input);
    }

    #[test]
    fn run_effect_chain_crossfades_enable_toggle_without_large_jump() {
        use crate::dsp::effects::{AudioEffect, GainEffect};

        let mut effect = GainEffect::default();
        effect.enabled = true;
        effect.settings.gain = 0.0;
        let mut effects = vec![AudioEffect::Gain(effect)];
        let input = vec![1.0_f32; 240 * 2];
        let mut scratch_a = Vec::new();
        let mut scratch_b = Vec::new();
        let mut enable_fades = vec![Some(EffectEnableFade::new(1.0, false, 240))];

        run_effect_chain(EffectChainRun {
            effects: &mut effects,
            input: &input,
            context: &context(),
            drain: false,
            scratch_a: &mut scratch_a,
            scratch_b: &mut scratch_b,
            observer: None,
            enable_fades: Some(&mut enable_fades),
        });

        let mut max_step = 0.0_f32;
        for frame in scratch_a.as_chunks::<2>().0 {
            max_step = max_step.max((frame[0] - frame[1]).abs());
        }
        assert!(max_step <= 1.0e-6);

        let mut prev = scratch_a[0];
        let mut largest_delta = 0.0_f32;
        for frame in scratch_a.as_chunks::<2>().0.iter().skip(1) {
            largest_delta = largest_delta.max((frame[0] - prev).abs());
            prev = frame[0];
        }
        assert!(largest_delta < 0.01);
        assert!(!audio_effect_enabled(&effects[0]));
        assert!(enable_fades[0].is_none());
    }

    #[test]
    fn run_effect_chain_observer_does_not_change_audio_output() {
        use crate::dsp::effects::{AudioEffect, GainEffect};

        struct RecordingObserver {
            visits: usize,
        }

        impl EffectChainObserver for RecordingObserver {
            fn before_effect(
                &mut self,
                _effect_index: usize,
                _effect: &AudioEffect,
                _input: &[f32],
                _channels: usize,
            ) {
                self.visits += 1;
            }

            fn after_effect(
                &mut self,
                _effect_index: usize,
                _effect: &AudioEffect,
                _output: &[f32],
                _channels: usize,
            ) {
                self.visits += 1;
            }
        }

        let mut gain = GainEffect::default();
        gain.enabled = true;
        gain.settings.gain = 2.0;
        let input = vec![0.25_f32, -0.25, 0.5, -0.5];
        let mut without_observer = vec![AudioEffect::Gain(gain.clone())];
        let mut with_observer = vec![AudioEffect::Gain(gain)];
        let mut expected_a = Vec::new();
        let mut expected_b = Vec::new();
        let mut observed_a = Vec::new();
        let mut observed_b = Vec::new();
        let mut observer = RecordingObserver { visits: 0 };

        run_effect_chain(EffectChainRun {
            effects: &mut without_observer,
            input: &input,
            context: &context(),
            drain: false,
            scratch_a: &mut expected_a,
            scratch_b: &mut expected_b,
            observer: None,
            enable_fades: None,
        });
        run_effect_chain(EffectChainRun {
            effects: &mut with_observer,
            input: &input,
            context: &context(),
            drain: false,
            scratch_a: &mut observed_a,
            scratch_b: &mut observed_b,
            observer: Some(&mut observer),
            enable_fades: None,
        });

        assert_eq!(expected_a, observed_a);
        assert_eq!(observer.visits, 2);
    }
}
