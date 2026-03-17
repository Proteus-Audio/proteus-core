//! Algorithmic reverb with smooth diffusion.
//!
//! This reverb uses a Schroeder-inspired layout with extra diffusion stages:
//! 1) A pre-delay to separate the direct sound from the reverb onset.
//! 2) A short series of input allpass diffusers to spread transients.
//! 3) A bank of parallel lowpass feedback comb filters to build decay.
//! 4) A short set of output allpass diffusers plus tone shaping for a softer tail.
//!
//! Each channel keeps an independent reverb lane (with small decorrelated
//! delay offsets) to avoid cross-channel ringing from processing interleaved
//! samples through one shared delay network.
//!
//! # Tuning Notes
//! - For a warmer/deeper tail, increase `room_size_ms` first, then `decay`.
//! - Raise `damping` (`~0.45..0.65`) to reduce metallic high-frequency ringing.
//! - Keep `diffusion` moderately high (`~0.65..0.80`) for density without excessive smear.
//! - Use lower `mix` for insert use on full mixes; higher `mix` works better on sends/auxes.
//!
//! DSP primitives (`DelayLine`, `CombFilter`, `AllpassFilter`, etc.) and the
//! runtime state struct live in the private `primitives` module.

use serde::{Deserialize, Serialize};

use super::EffectContext;

mod primitives;

use primitives::{delay_samples, DiffusionReverbState};

const DEFAULT_PRE_DELAY_MS: u64 = 12;
const DEFAULT_ROOM_SIZE_MS: u64 = 48;
const DEFAULT_DECAY: f32 = 0.72;
const DEFAULT_DAMPING: f32 = 0.35;
const DEFAULT_DIFFUSION: f32 = 0.72;
const MAX_DECAY: f32 = 0.98;
const MAX_DAMPING: f32 = 0.99;
const MAX_DIFFUSION: f32 = 0.9;

const COMB_TUNING_MULTIPLIERS: [f32; 8] = [1.0, 1.09, 1.2, 1.33, 1.47, 1.63, 1.82, 2.03];
const INPUT_ALLPASS_TUNING_MULTIPLIERS: [f32; 3] = [0.07, 0.11, 0.17];
const OUTPUT_ALLPASS_TUNING_MULTIPLIERS: [f32; 3] = [0.28, 0.41, 0.57];

/// Serialized configuration for the diffusion reverb.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffusionReverbSettings {
    /// Pre-delay time in milliseconds.
    ///
    /// Increases the gap between the dry signal and the reverb onset, which
    /// improves clarity and perceived depth.
    pub pre_delay_ms: u64,
    /// Base delay time in milliseconds that scales the internal diffuser/comb timings.
    ///
    /// Larger values generally produce a deeper, larger-sounding space.
    pub room_size_ms: u64,
    /// Feedback amount for the comb filters.
    ///
    /// Higher values increase reverb time. Excessively high values can make the
    /// tail ring or feel static, especially on bright sources.
    pub decay: f32,
    /// Lowpass damping applied inside each comb feedback path.
    ///
    /// Higher values darken the tail and reduce metallic brightness.
    pub damping: f32,
    /// Feedback amount for the allpass diffusers.
    ///
    /// Higher values increase density and smoothness, but can blur transients.
    pub diffusion: f32,
}

impl DiffusionReverbSettings {
    /// Create diffusion reverb settings with validation and clamping.
    ///
    /// # Arguments
    /// - `pre_delay_ms`: Pre-delay time in milliseconds.
    /// - `room_size_ms`: Base delay for the internal timing network in milliseconds.
    /// - `decay`: Comb feedback gain in the range `[0.0, 1.0)`.
    /// - `damping`: Comb feedback lowpass damping in the range `[0.0, 1.0)`.
    /// - `diffusion`: Diffuser allpass feedback gain in the range `[0.0, 1.0)`.
    ///
    /// # Returns
    /// The validated settings.
    pub fn new(
        pre_delay_ms: u64,
        room_size_ms: u64,
        decay: f32,
        damping: f32,
        diffusion: f32,
    ) -> Self {
        Self {
            pre_delay_ms: pre_delay_ms.clamp(0, u64::MAX),
            room_size_ms: room_size_ms.clamp(0, u64::MAX),
            decay: decay.clamp(0.0, MAX_DECAY),
            damping: damping.clamp(0.0, MAX_DAMPING),
            diffusion: diffusion.clamp(0.0, MAX_DIFFUSION),
        }
    }

    fn decay(&self) -> f32 {
        self.decay.clamp(0.0, MAX_DECAY)
    }

    fn damping(&self) -> f32 {
        self.damping.clamp(0.0, MAX_DAMPING)
    }

    fn diffusion(&self) -> f32 {
        self.diffusion.clamp(0.0, MAX_DIFFUSION)
    }
}

impl Default for DiffusionReverbSettings {
    fn default() -> Self {
        Self {
            pre_delay_ms: DEFAULT_PRE_DELAY_MS,
            room_size_ms: DEFAULT_ROOM_SIZE_MS,
            decay: DEFAULT_DECAY,
            damping: DEFAULT_DAMPING,
            diffusion: DEFAULT_DIFFUSION,
        }
    }
}

/// Diffusion reverb effect with a Schroeder-inspired, multi-stage topology.
///
/// Internally this maintains one decorrelated reverb lane per output channel to
/// avoid left/right cross-coupled ringing when processing interleaved buffers.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffusionReverbEffect {
    /// Whether the effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Dry/wet mix ratio (0.0 = fully dry, 1.0 = fully wet).
    #[serde(alias = "dry_wet", alias = "wet_dry")]
    pub mix: f32,
    /// Diffusion reverb parameters controlling decay, diffusion, and room size.
    #[serde(flatten)]
    pub settings: DiffusionReverbSettings,
    #[serde(skip)]
    state: Option<DiffusionReverbState>,
    // Guards the `drain=true && samples.is_empty()` path so the mix thread does
    // not receive an endless sequence of synthetic tail chunks.
    #[serde(skip)]
    tail_drained: bool,
}

impl Default for DiffusionReverbEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            mix: 0.0,
            settings: DiffusionReverbSettings::default(),
            state: None,
            tail_drained: false,
        }
    }
}

impl std::fmt::Debug for DiffusionReverbEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffusionReverbEffect")
            .field("enabled", &self.enabled)
            .field("mix", &self.mix)
            .field("settings", &self.settings)
            .finish()
    }
}

impl crate::dsp::effects::core::DspEffect for DiffusionReverbEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            return samples.to_vec();
        }

        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        if samples.is_empty() {
            if drain {
                // Drain is a one-shot flush. The mix runtime may poll drain
                // repeatedly after sources finish, so subsequent calls must be
                // empty once the internal tail has been emitted.
                if self.tail_drained {
                    return Vec::new();
                }
                self.tail_drained = true;
                return state.drain_tail(
                    self.settings.decay(),
                    self.settings.damping(),
                    self.settings.diffusion(),
                );
            }
            return Vec::new();
        }

        // Any real input means the effect is active again; allow a future tail
        // flush when playback ends.
        self.tail_drained = false;

        let mix = self.mix.clamp(0.0, 1.0);
        let mut output = Vec::with_capacity(samples.len());
        state.process_samples(
            samples,
            mix,
            self.settings.decay(),
            self.settings.damping(),
            self.settings.diffusion(),
            &mut output,
        );
        output
    }

    fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<f32>,
        context: &EffectContext,
        drain: bool,
    ) {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            output.extend_from_slice(input);
            return;
        }
        let Some(state) = self.state.as_mut() else {
            output.extend_from_slice(input);
            return;
        };
        if input.is_empty() {
            if drain {
                if self.tail_drained {
                    return;
                }
                self.tail_drained = true;
                let tail = state.drain_tail(
                    self.settings.decay(),
                    self.settings.damping(),
                    self.settings.diffusion(),
                );
                output.extend(tail);
            }
            return;
        }
        self.tail_drained = false;
        let mix = self.mix.clamp(0.0, 1.0);
        state.process_samples(
            input,
            mix,
            self.settings.decay(),
            self.settings.damping(),
            self.settings.diffusion(),
            output,
        );
    }

    fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
        self.tail_drained = false;
    }
}

impl DiffusionReverbEffect {
    /// Create a new diffusion reverb effect with default settings.
    ///
    /// # Arguments
    /// - `mix`: Wet/dry mix in the range `[0.0, 1.0]`.
    ///
    /// # Returns
    /// The configured diffusion reverb effect.
    pub fn new(mix: f32) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Mutable access to the diffusion reverb settings.
    ///
    /// Changing timing-related fields (`pre_delay_ms`, `room_size_ms`) will cause
    /// the internal state to be rebuilt on the next `process` call.
    pub fn settings_mut(&mut self) -> &mut DiffusionReverbSettings {
        &mut self.settings
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let pre_delay_samples = delay_samples(context.sample_rate(), self.settings.pre_delay_ms);
        let room_size_samples = delay_samples(context.sample_rate(), self.settings.room_size_ms);
        let tuning = Tuning::new(pre_delay_samples, room_size_samples);
        let channels = context.channels().max(1);
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| state.tuning != tuning || state.channels != channels)
            .unwrap_or(true);
        if needs_reset {
            // Timing and channel count define delay-line topology, so rebuild
            // when those change instead of attempting partial mutation.
            self.state = Some(DiffusionReverbState::new(tuning, channels));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Derived delay lengths for the internal reverb topology.
///
/// This is computed from user settings and sample rate, then optionally
/// decorrelated per channel so each lane rings differently.
pub(super) struct Tuning {
    pub(super) pre_delay_samples: usize,
    pub(super) comb_samples: [usize; 8],
    pub(super) input_allpass_samples: [usize; 3],
    pub(super) output_allpass_samples: [usize; 3],
    pub(super) max_delay: usize,
}

impl Tuning {
    /// Build tuning values from pre-delay and room-size sample counts.
    fn new(pre_delay_samples: usize, room_size_samples: usize) -> Self {
        let comb_samples = COMB_TUNING_MULTIPLIERS
            .map(|multiplier| (room_size_samples as f32 * multiplier).round() as usize);
        let input_allpass_samples = INPUT_ALLPASS_TUNING_MULTIPLIERS
            .map(|multiplier| (room_size_samples as f32 * multiplier).round() as usize);
        let output_allpass_samples = OUTPUT_ALLPASS_TUNING_MULTIPLIERS
            .map(|multiplier| (room_size_samples as f32 * multiplier).round() as usize);
        let max_delay = comb_samples
            .iter()
            .copied()
            .chain(input_allpass_samples.iter().copied())
            .chain(output_allpass_samples.iter().copied())
            .chain([pre_delay_samples])
            .max()
            .unwrap_or(1)
            .max(1);
        Self {
            pre_delay_samples: pre_delay_samples.max(1),
            comb_samples: comb_samples.map(|value| value.max(1)),
            input_allpass_samples: input_allpass_samples.map(|value| value.max(1)),
            output_allpass_samples: output_allpass_samples.map(|value| value.max(1)),
            max_delay,
        }
    }

    /// Apply small channel-dependent offsets to reduce stereo correlation.
    ///
    /// This preserves the general room scale while changing exact modal spacing
    /// across channels.
    pub(super) fn decorrelated(self, channel_index: usize) -> Self {
        if channel_index == 0 {
            return self;
        }
        let channel_step = channel_index;
        let mut tuned = self;
        tuned.pre_delay_samples = tuned.pre_delay_samples.saturating_add(channel_step * 3);
        tuned.comb_samples = tuned
            .comb_samples
            .iter()
            .enumerate()
            .map(|(i, &len)| len.saturating_add(channel_step * (5 + i * 2)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or(tuned.comb_samples);
        tuned.input_allpass_samples = tuned
            .input_allpass_samples
            .iter()
            .enumerate()
            .map(|(i, &len)| len.saturating_add(channel_step * (2 + i)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or(tuned.input_allpass_samples);
        tuned.output_allpass_samples = tuned
            .output_allpass_samples
            .iter()
            .enumerate()
            .map(|(i, &len)| len.saturating_add(channel_step * (3 + i)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or(tuned.output_allpass_samples);
        tuned.max_delay = tuned
            .comb_samples
            .iter()
            .copied()
            .chain(tuned.input_allpass_samples.iter().copied())
            .chain(tuned.output_allpass_samples.iter().copied())
            .chain([tuned.pre_delay_samples])
            .max()
            .unwrap_or(1)
            .max(1);
        tuned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::effects::core::DspEffect;

    fn context() -> EffectContext {
        EffectContext::new(48_000, 2, None, None, -60.0).unwrap()
    }

    #[test]
    fn delay_samples_returns_zero_for_zero_duration() {
        assert_eq!(delay_samples(48_000, 0), 0);
    }

    #[test]
    fn diffusion_settings_new_clamps_values() {
        let settings = DiffusionReverbSettings::new(0, 0, 10.0, 10.0, 10.0);
        assert!(settings.decay <= MAX_DECAY);
        assert!(settings.damping <= MAX_DAMPING);
        assert!(settings.diffusion <= MAX_DIFFUSION);
    }

    #[test]
    fn diffusion_reverb_passthrough_when_mix_is_zero() {
        let mut effect = DiffusionReverbEffect::new(0.0);
        let input = vec![0.1_f32, -0.1, 0.2, -0.2];
        let output = effect.process(&input, &context(), false);
        assert_eq!(output, input);
    }

    #[test]
    fn diffusion_reverb_process_preserves_length() {
        let mut effect = DiffusionReverbEffect::new(0.4);
        let input = vec![0.1_f32, -0.1, 0.2, -0.2];
        let output = effect.process(&input, &context(), false);
        assert_eq!(output.len(), input.len());
    }
}
