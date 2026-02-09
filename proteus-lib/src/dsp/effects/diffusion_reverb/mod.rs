//! Algorithmic reverb with smooth diffusion.
//!
//! This reverb follows a classic Schroeder-style layout:
//! 1) A pre-delay to separate the direct sound from the reverb onset.
//! 2) A set of parallel lowpass feedback comb filters to build decay.
//! 3) A pair of series allpass filters to smooth the diffusion.
//!
//! The comb filters create the overall decay envelope while the allpass
//! stages smear transients so the tail sounds dense instead of grainy.

use log::info;
use serde::{Deserialize, Serialize};

use super::EffectContext;

const DEFAULT_PRE_DELAY_MS: u64 = 12;
const DEFAULT_ROOM_SIZE_MS: u64 = 48;
const DEFAULT_DECAY: f32 = 0.72;
const DEFAULT_DAMPING: f32 = 0.35;
const DEFAULT_DIFFUSION: f32 = 0.72;
const MAX_DECAY: f32 = 0.98;
const MAX_DAMPING: f32 = 0.99;
const MAX_DIFFUSION: f32 = 0.9;

const COMB_TUNING_MULTIPLIERS: [f32; 4] = [1.0, 1.33, 1.58, 1.91];
const ALLPASS_TUNING_MULTIPLIERS: [f32; 2] = [0.28, 0.52];

/// Serialized configuration for the diffusion reverb.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffusionReverbSettings {
    /// Pre-delay time in milliseconds.
    pub pre_delay_ms: u64,
    /// Base delay time in milliseconds that scales the comb filters.
    pub room_size_ms: u64,
    /// Feedback amount for the comb filters. Higher values mean longer decay.
    pub decay: f32,
    /// Lowpass damping applied inside the comb feedback path.
    pub damping: f32,
    /// Feedback amount for the allpass diffusers. Higher values increase density.
    pub diffusion: f32,
}

impl DiffusionReverbSettings {
    /// Create diffusion reverb settings with basic validation.
    ///
    /// # Arguments
    /// - `pre_delay_ms`: Pre-delay time in milliseconds.
    /// - `room_size_ms`: Base delay for the comb filters in milliseconds.
    /// - `decay`: Comb feedback gain in the range `[0.0, 1.0)`.
    /// - `damping`: Lowpass damping factor in the range `[0.0, 1.0)`.
    /// - `diffusion`: Allpass feedback gain in the range `[0.0, 1.0)`.
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

/// Diffusion reverb effect (pre-delay + combs + allpass diffusion).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffusionReverbEffect {
    pub enabled: bool,
    #[serde(alias = "dry_wet", alias = "wet_dry")]
    pub mix: f32,
    #[serde(flatten)]
    pub settings: DiffusionReverbSettings,
    #[serde(skip)]
    state: Option<DiffusionReverbState>,
}

impl Default for DiffusionReverbEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            mix: 0.0,
            settings: DiffusionReverbSettings::default(),
            state: None,
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

impl DiffusionReverbEffect {
    /// Create a new diffusion reverb effect.
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

    /// Process interleaved samples through the diffusion reverb.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: When true, flush buffered tail data.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
            return samples.to_vec();
        }

        // If an impulse response is configured, skip algorithmic reverb in favor of convolution.
        if context.impulse_response_spec.is_some() {
            return samples.to_vec();
        }

        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        if samples.is_empty() {
            if drain {
                return state.drain_tail(
                    self.settings.decay(),
                    self.settings.damping(),
                    self.settings.diffusion(),
                );
            }
            return Vec::new();
        }

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

    /// Reset any internal state buffers.
    ///
    /// # Returns
    /// Nothing.
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }

    /// Mutable access to the diffusion reverb settings.
    pub fn settings_mut(&mut self) -> &mut DiffusionReverbSettings {
        &mut self.settings
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let pre_delay_samples = delay_samples(
            context.sample_rate,
            context.channels,
            self.settings.pre_delay_ms,
        );
        let room_size_samples = delay_samples(
            context.sample_rate,
            context.channels,
            self.settings.room_size_ms,
        );
        let tuning = Tuning::new(pre_delay_samples, room_size_samples);
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| state.tuning != tuning)
            .unwrap_or(true);
        if needs_reset {
            self.state = Some(DiffusionReverbState::new(tuning));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tuning {
    pre_delay_samples: usize,
    comb_samples: [usize; 4],
    allpass_samples: [usize; 2],
    max_delay: usize,
}

impl Tuning {
    fn new(pre_delay_samples: usize, room_size_samples: usize) -> Self {
        let comb_samples = COMB_TUNING_MULTIPLIERS
            .map(|multiplier| (room_size_samples as f32 * multiplier).round() as usize);
        let allpass_samples = ALLPASS_TUNING_MULTIPLIERS
            .map(|multiplier| (room_size_samples as f32 * multiplier).round() as usize);
        let max_delay = comb_samples
            .iter()
            .copied()
            .chain(allpass_samples.iter().copied())
            .chain([pre_delay_samples])
            .max()
            .unwrap_or(1)
            .max(1);
        Self {
            pre_delay_samples: pre_delay_samples.max(1),
            comb_samples: comb_samples.map(|value| value.max(1)),
            allpass_samples: allpass_samples.map(|value| value.max(1)),
            max_delay,
        }
    }
}

#[derive(Clone)]
struct DiffusionReverbState {
    tuning: Tuning,
    pre_delay: DelayLine,
    combs: [CombFilter; 4],
    allpass: [AllpassFilter; 2],
}

impl DiffusionReverbState {
    fn new(tuning: Tuning) -> Self {
        info!("Using Diffusion Reverb!");
        Self {
            tuning,
            pre_delay: DelayLine::new(tuning.pre_delay_samples),
            combs: [
                CombFilter::new(tuning.comb_samples[0]),
                CombFilter::new(tuning.comb_samples[1]),
                CombFilter::new(tuning.comb_samples[2]),
                CombFilter::new(tuning.comb_samples[3]),
            ],
            allpass: [
                AllpassFilter::new(tuning.allpass_samples[0]),
                AllpassFilter::new(tuning.allpass_samples[1]),
            ],
        }
    }

    fn reset(&mut self) {
        self.pre_delay.reset();
        for comb in &mut self.combs {
            comb.reset();
        }
        for allpass in &mut self.allpass {
            allpass.reset();
        }
    }

    fn process_samples(
        &mut self,
        samples: &[f32],
        mix: f32,
        decay: f32,
        damping: f32,
        diffusion: f32,
        out: &mut Vec<f32>,
    ) {
        for &sample in samples {
            let delayed = self.pre_delay.process(sample);
            let mut comb_sum = 0.0;
            for comb in &mut self.combs {
                comb_sum += comb.process(delayed, decay, damping);
            }
            let mut wet = comb_sum * 0.25;
            for allpass in &mut self.allpass {
                wet = allpass.process(wet, diffusion);
            }
            let output = sample * (1.0 - mix) + wet * mix;
            out.push(output);
        }
    }

    fn drain_tail(&mut self, decay: f32, damping: f32, diffusion: f32) -> Vec<f32> {
        let tail_samples = self.tuning.max_delay.saturating_mul(4).max(1);
        let mut out = Vec::with_capacity(tail_samples);
        for _ in 0..tail_samples {
            let delayed = self.pre_delay.process(0.0);
            let mut comb_sum = 0.0;
            for comb in &mut self.combs {
                comb_sum += comb.process(delayed, decay, damping);
            }
            let mut wet = comb_sum * 0.25;
            for allpass in &mut self.allpass {
                wet = allpass.process(wet, diffusion);
            }
            out.push(wet);
        }
        out
    }
}

#[derive(Clone)]
struct DelayLine {
    buffer: Vec<f32>,
    index: usize,
}

impl DelayLine {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.buffer[self.index] = input;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }
}

#[derive(Clone)]
struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    lowpass: f32,
}

impl CombFilter {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
            lowpass: 0.0,
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.lowpass = 0.0;
    }

    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let delayed = self.buffer[self.index];
        self.lowpass = delayed * (1.0 - damping) + self.lowpass * damping;
        let output = self.lowpass;
        self.buffer[self.index] = input + output * feedback;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }
}

#[derive(Clone)]
struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
}

impl AllpassFilter {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    fn process(&mut self, input: f32, feedback: f32) -> f32 {
        let delayed = self.buffer[self.index];
        let output = delayed - feedback * input;
        self.buffer[self.index] = input + delayed * feedback;
        self.index += 1;
        if self.index >= self.buffer.len() {
            self.index = 0;
        }
        output
    }
}

fn delay_samples(sample_rate: u32, channels: usize, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let ns = duration_ms.saturating_mul(1_000_000);
    let samples = ns.saturating_mul(sample_rate as u64) / 1_000_000_000 * channels as u64;
    samples as usize
}
