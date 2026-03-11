//! DSP primitives and runtime state for the diffusion reverb.
//!
//! Contains the per-channel reverb lane, its component filters, the runtime
//! state struct that owns the lane collection, and the `delay_samples` helper.

// Upper bound for synthetic silence-fed tail flushing.
const DRAIN_MAX_TAIL_MULTIPLIER: usize = 64;
// Treat frames below this absolute amplitude as silence for tail termination.
const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;
// Require a short run of silent frames before declaring the tail finished.
const DRAIN_SILENT_FRAMES_TO_STOP: usize = 128;

#[derive(Clone)]
/// Runtime state for the diffusion reverb effect.
///
/// Holds one [`ReverbLane`] per channel and routes interleaved frames into the
/// matching lane.
pub(super) struct DiffusionReverbState {
    pub(super) tuning: super::Tuning,
    pub(super) channels: usize,
    lanes: Vec<ReverbLane>,
}

impl DiffusionReverbState {
    /// Create a new state instance for the current tuning and channel count.
    pub(super) fn new(tuning: super::Tuning, channels: usize) -> Self {
        log::info!("Using Diffusion Reverb!");
        let lanes = (0..channels)
            .map(|channel| ReverbLane::new(tuning.decorrelated(channel)))
            .collect();
        Self { tuning, channels, lanes }
    }

    /// Reset all channel lanes.
    pub(super) fn reset(&mut self) {
        for lane in &mut self.lanes {
            lane.reset();
        }
    }

    /// Process interleaved samples into the provided output buffer.
    ///
    /// The `diffusion` control is mapped to separate input/output diffuser
    /// strengths so early smearing and late-tail smoothing can be balanced.
    pub(super) fn process_samples(
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
    pub(super) fn drain_tail(&mut self, decay: f32, damping: f32, diffusion: f32) -> Vec<f32> {
        let input_diffusion = (0.25 + diffusion * 0.35).clamp(0.1, 0.7);
        let output_diffusion = (0.2 + diffusion * 0.45).clamp(0.1, 0.8);
        let max_tail_frames = self
            .tuning
            .max_delay
            .saturating_mul(DRAIN_MAX_TAIL_MULTIPLIER)
            .max(1);
        let mut out = Vec::with_capacity(max_tail_frames.saturating_mul(self.channels));
        let mut trailing_silent_frames = 0usize;
        for _ in 0..max_tail_frames {
            // Track frame start so we can drop the final fully-silent run rather
            // than returning a padded block of near-zero samples.
            let frame_start = out.len();
            let mut max_abs = 0.0_f32;
            for lane in &mut self.lanes {
                let wet =
                    lane.process_sample(0.0, decay, damping, input_diffusion, output_diffusion);
                max_abs = max_abs.max(wet.abs());
                out.push(wet);
            }
            if max_abs <= DRAIN_SILENCE_EPSILON {
                trailing_silent_frames = trailing_silent_frames.saturating_add(1);
            } else {
                trailing_silent_frames = 0;
            }
            if trailing_silent_frames >= DRAIN_SILENT_FRAMES_TO_STOP {
                out.truncate(frame_start);
                break;
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
    fn new(tuning: super::Tuning) -> Self {
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
pub(super) fn delay_samples(sample_rate: u32, duration_ms: u64) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let ns = duration_ms.saturating_mul(1_000_000);
    let samples = ns.saturating_mul(sample_rate as u64) / 1_000_000_000;
    samples as usize
}
