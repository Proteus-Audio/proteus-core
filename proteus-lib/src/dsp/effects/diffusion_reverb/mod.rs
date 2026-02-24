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

    /// Process interleaved samples through the diffusion reverb.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: When true and `samples` is empty, flush buffered tail data.
    ///
    /// # Returns
    /// Processed interleaved samples.
    ///
    /// # Notes
    /// - Input is treated as interleaved frames using `context.channels`.
    /// - If the buffer length is not frame-aligned, trailing samples are passed through unchanged.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.mix <= 0.0 {
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

    /// Reset all internal delay/filter buffers and drop the current state.
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
    ///
    /// Changing timing-related fields (`pre_delay_ms`, `room_size_ms`) will cause
    /// the internal state to be rebuilt on the next call to [`Self::process`].
    pub fn settings_mut(&mut self) -> &mut DiffusionReverbSettings {
        &mut self.settings
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let pre_delay_samples = delay_samples(context.sample_rate, self.settings.pre_delay_ms);
        let room_size_samples = delay_samples(context.sample_rate, self.settings.room_size_ms);
        let tuning = Tuning::new(pre_delay_samples, room_size_samples);
        let channels = context.channels.max(1);
        let needs_reset = self
            .state
            .as_ref()
            .map(|state| state.tuning != tuning || state.channels != channels)
            .unwrap_or(true);
        if needs_reset {
            self.state = Some(DiffusionReverbState::new(tuning, channels));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Derived delay lengths for the internal reverb topology.
///
/// This is computed from user settings and sample rate, then optionally
/// decorrelated per channel so each lane rings differently.
struct Tuning {
    pre_delay_samples: usize,
    comb_samples: [usize; 8],
    input_allpass_samples: [usize; 3],
    output_allpass_samples: [usize; 3],
    max_delay: usize,
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
    fn decorrelated(self, channel_index: usize) -> Self {
        if channel_index == 0 {
            return self;
        }
        let channel_step = channel_index as usize;
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

#[derive(Clone)]
/// Runtime state for the diffusion reverb effect.
///
/// Holds one [`ReverbLane`] per channel and routes interleaved frames into the
/// matching lane.
struct DiffusionReverbState {
    tuning: Tuning,
    channels: usize,
    lanes: Vec<ReverbLane>,
}

impl DiffusionReverbState {
    /// Create a new state instance for the current tuning and channel count.
    fn new(tuning: Tuning, channels: usize) -> Self {
        info!("Using Diffusion Reverb!");
        let lanes = (0..channels)
            .map(|channel| ReverbLane::new(tuning.decorrelated(channel)))
            .collect();
        Self {
            tuning,
            channels,
            lanes,
        }
    }

    /// Reset all channel lanes.
    fn reset(&mut self) {
        for lane in &mut self.lanes {
            lane.reset();
        }
    }

    /// Process interleaved samples into the provided output buffer.
    ///
    /// The `diffusion` control is mapped to separate input/output diffuser
    /// strengths so early smearing and late-tail smoothing can be balanced.
    fn process_samples(
        &mut self,
        samples: &[f32],
        mix: f32,
        decay: f32,
        damping: f32,
        diffusion: f32,
        out: &mut Vec<f32>,
    ) {
        let channels = self.channels.max(1);
        let input_diffusion = (0.25 + diffusion * 0.35).clamp(0.1, 0.7);
        let output_diffusion = (0.2 + diffusion * 0.45).clamp(0.1, 0.8);

        for frame in samples.chunks_exact(channels) {
            for (channel, &sample) in frame.iter().enumerate() {
                let wet = self.lanes[channel].process_sample(
                    sample,
                    decay,
                    damping,
                    input_diffusion,
                    output_diffusion,
                );
                out.push(sample * (1.0 - mix) + wet * mix);
            }
        }

        let remainder = samples.len() % channels;
        if remainder != 0 {
            let start = samples.len() - remainder;
            out.extend_from_slice(&samples[start..]);
        }
    }

    /// Drain the buffered reverb tail by feeding silence through all lanes.
    fn drain_tail(&mut self, decay: f32, damping: f32, diffusion: f32) -> Vec<f32> {
        let input_diffusion = (0.25 + diffusion * 0.35).clamp(0.1, 0.7);
        let output_diffusion = (0.2 + diffusion * 0.45).clamp(0.1, 0.8);
        let tail_frames = self.tuning.max_delay.saturating_mul(8).max(1);
        let mut out = Vec::with_capacity(tail_frames.saturating_mul(self.channels));
        for _ in 0..tail_frames {
            for lane in &mut self.lanes {
                let wet =
                    lane.process_sample(0.0, decay, damping, input_diffusion, output_diffusion);
                out.push(wet);
            }
        }
        out
    }
}

#[derive(Clone)]
/// One complete mono reverb lane used by a single output channel.
///
/// Layout: pre-delay -> input allpass diffusion -> comb bank ->
/// output allpass diffusion -> wet tone lowpass.
struct ReverbLane {
    pre_delay: DelayLine,
    input_allpass: [AllpassFilter; 3],
    combs: [CombFilter; 8],
    output_allpass: [AllpassFilter; 3],
    wet_tone: OnePoleLowpass,
}

impl ReverbLane {
    /// Build a lane using a channel-specific tuning table.
    fn new(tuning: Tuning) -> Self {
        Self {
            pre_delay: DelayLine::new(tuning.pre_delay_samples),
            input_allpass: [
                AllpassFilter::new(tuning.input_allpass_samples[0]),
                AllpassFilter::new(tuning.input_allpass_samples[1]),
                AllpassFilter::new(tuning.input_allpass_samples[2]),
            ],
            combs: [
                CombFilter::new(tuning.comb_samples[0]),
                CombFilter::new(tuning.comb_samples[1]),
                CombFilter::new(tuning.comb_samples[2]),
                CombFilter::new(tuning.comb_samples[3]),
                CombFilter::new(tuning.comb_samples[4]),
                CombFilter::new(tuning.comb_samples[5]),
                CombFilter::new(tuning.comb_samples[6]),
                CombFilter::new(tuning.comb_samples[7]),
            ],
            output_allpass: [
                AllpassFilter::new(tuning.output_allpass_samples[0]),
                AllpassFilter::new(tuning.output_allpass_samples[1]),
                AllpassFilter::new(tuning.output_allpass_samples[2]),
            ],
            wet_tone: OnePoleLowpass::default(),
        }
    }

    /// Reset the lane and all internal filters/delays.
    fn reset(&mut self) {
        self.pre_delay.reset();
        for allpass in &mut self.input_allpass {
            allpass.reset();
        }
        for comb in &mut self.combs {
            comb.reset();
        }
        for allpass in &mut self.output_allpass {
            allpass.reset();
        }
        self.wet_tone.reset();
    }

    /// Process one mono sample through the lane.
    ///
    /// `input_diffusion` and `output_diffusion` are derived from the user-facing
    /// diffusion control and intentionally use different ranges to keep attacks
    /// clear while still smoothing the late tail.
    fn process_sample(
        &mut self,
        input: f32,
        decay: f32,
        damping: f32,
        input_diffusion: f32,
        output_diffusion: f32,
    ) -> f32 {
        let mut x = self.pre_delay.process(input);
        for allpass in &mut self.input_allpass {
            x = allpass.process(x, input_diffusion);
        }

        let mut comb_sum = 0.0;
        for comb in &mut self.combs {
            comb_sum += comb.process(x, decay, damping);
        }

        let mut wet = comb_sum / self.combs.len() as f32;
        for allpass in &mut self.output_allpass {
            wet = allpass.process(wet, output_diffusion);
        }

        // Soften high-frequency ringing in the late tail.
        let tone_smoothing = (0.55 + damping * 0.35).clamp(0.2, 0.95);
        self.wet_tone.process(wet, tone_smoothing)
    }
}

#[derive(Clone)]
/// Fixed-length circular delay line.
struct DelayLine {
    buffer: Vec<f32>,
    index: usize,
}

impl DelayLine {
    /// Create a delay line with at least one sample of storage.
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
        }
    }

    /// Clear the delay buffer and rewind the write index.
    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    /// Push one sample and return the delayed sample at the current tap.
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
/// Lowpass-feedback comb filter used to build the late reverb decay.
///
/// The internal lowpass reduces high-frequency build-up in the feedback loop.
struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    lowpass: f32,
}

impl CombFilter {
    /// Create a comb filter with a fixed delay length.
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
            lowpass: 0.0,
        }
    }

    /// Clear delay and lowpass state.
    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.lowpass = 0.0;
    }

    /// Process one sample through the comb filter.
    ///
    /// # Arguments
    /// - `input`: Dry/diffused input into the comb.
    /// - `feedback`: Feedback gain controlling decay time.
    /// - `damping`: One-pole lowpass smoothing in the feedback path.
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
/// Standard feedback allpass diffuser.
struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
}

impl AllpassFilter {
    /// Create an allpass filter with a fixed delay length.
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
        }
    }

    /// Clear the delay buffer and rewind the read/write position.
    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    /// Process one sample through the allpass diffuser.
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

#[derive(Clone, Default)]
/// One-pole lowpass used to slightly darken the wet output tail.
struct OnePoleLowpass {
    state: f32,
}

impl OnePoleLowpass {
    /// Reset the filter state.
    fn reset(&mut self) {
        self.state = 0.0;
    }

    /// Process one sample with the provided smoothing coefficient.
    ///
    /// `smoothing` near `1.0` produces a darker/slower response.
    fn process(&mut self, input: f32, smoothing: f32) -> f32 {
        self.state = input * (1.0 - smoothing) + self.state * smoothing;
        self.state
    }
}

/// Convert milliseconds to per-channel sample counts.
///
/// The returned value represents samples for a single channel lane (not the
/// total number of interleaved scalar values).
fn delay_samples(sample_rate: u32, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let ns = duration_ms.saturating_mul(1_000_000);
    let samples = ns.saturating_mul(sample_rate as u64) / 1_000_000_000;
    samples as usize
}
