//! Offline audio-effect benchmarks used by the CLI and performance investigations.
//!
//! This module deliberately benchmarks already-decoded, interleaved PCM.  Keeping
//! decoding and project loading outside the timing loop makes results comparable
//! between effects and ensures the reported time is DSP processing time.

use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::dsp::effects::{
    AudioEffect, CompressorEffect, ConvolutionReverbEffect, DelayReverbEffect,
    DiffusionReverbEffect, DistortionEffect, EffectContext, EffectContextError, GainEffect,
    HighPassFilterEffect, LimiterEffect, LowPassFilterEffect, MultibandEqEffect, PanEffect,
};

/// Controls how an offline audio benchmark is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioBenchmarkConfig {
    /// Sample rate of the interleaved input, in Hz.
    pub sample_rate: u32,
    /// Number of interleaved channels in the input.
    pub channels: usize,
    /// Number of timed runs for every case.
    pub iterations: usize,
    /// Untimed runs performed before every case.
    pub warmup_iterations: usize,
    /// Number of frames passed to an effect at a time.
    pub chunk_frames: usize,
}

impl Default for AudioBenchmarkConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            channels: 2,
            iterations: 5,
            warmup_iterations: 1,
            // This is the same order of magnitude as the playback mixer buffer,
            // while remaining practical for effects that retain internal state.
            chunk_frames: 1_024,
        }
    }
}

/// Error returned for invalid benchmark configuration.
#[derive(Debug, Clone)]
pub enum AudioBenchmarkError {
    /// The requested chunk contains no frames.
    ZeroChunkFrames,
    /// The supplied PCM does not end on a complete interleaved frame.
    PartialFrame { samples: usize, channels: usize },
    /// The effect context could not be constructed.
    InvalidContext(EffectContextError),
}

impl std::fmt::Display for AudioBenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroChunkFrames => write!(f, "benchmark chunk size must be at least one frame"),
            Self::PartialFrame { samples, channels } => write!(
                f,
                "benchmark input contains {} samples, which is not divisible by {} channels",
                samples, channels
            ),
            Self::InvalidContext(error) => write!(f, "invalid benchmark effect context: {error}"),
        }
    }
}

impl std::error::Error for AudioBenchmarkError {}

/// Aggregate timing for one benchmark case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenchmarkTiming {
    /// Sum of all measured runs.
    pub total: Duration,
    /// Mean duration of a measured run.
    pub average: Duration,
    /// Fastest measured run.
    pub minimum: Duration,
    /// Slowest measured run.
    pub maximum: Duration,
    /// Mean processing time divided by input audio duration. Values below 1 are real-time.
    pub real_time_factor: f64,
}

/// One no-effects, single-effect, or project-chain benchmark result.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBenchmarkCase {
    /// Human-readable case name.
    pub name: String,
    /// Number of effects in the benchmarked chain.
    pub effect_count: usize,
    /// Timing summary for this case.
    pub timing: BenchmarkTiming,
}

/// Result of benchmarking an interleaved PCM buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBenchmarkReport {
    /// Configuration used for this report.
    pub config: AudioBenchmarkConfig,
    /// Input length in interleaved samples.
    pub samples: usize,
    /// Input length in complete frames.
    pub frames: usize,
    /// Duration represented by the input PCM.
    pub audio_duration: Duration,
    /// Cases in report order: no effects, every built-in effect, then optional project chain.
    pub cases: Vec<AudioBenchmarkCase>,
}

impl AudioBenchmarkReport {
    /// Render a portable Markdown report suitable for stdout or a `.md` file.
    pub fn to_markdown(&self, input_label: &str) -> String {
        let mut report = format!(
            "# Proteus audio benchmark\n\n\
             - Input: `{input_label}`\n\
             - PCM: {} frames, {} channels, {} Hz ({:.3} s)\n\
             - Timed iterations per case: {}\n\
             - Warmup iterations per case: {}\n\
             - Processing chunk: {} frames\n\n\
             | Case | Effects | Total (ms) | Average (ms) | Min (ms) | Max (ms) | Real-time factor |\n\
             | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n",
            self.frames,
            self.config.channels,
            self.config.sample_rate,
            self.audio_duration.as_secs_f64(),
            self.config.iterations.max(1),
            self.config.warmup_iterations,
            self.config.chunk_frames,
        );
        for case in &self.cases {
            report.push_str(&format!(
                "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3}x |\n",
                case.name,
                case.effect_count,
                milliseconds(case.timing.total),
                milliseconds(case.timing.average),
                milliseconds(case.timing.minimum),
                milliseconds(case.timing.maximum),
                case.timing.real_time_factor,
            ));
        }
        report
    }
}

/// Return one enabled default instance of every effect compiled into Proteus.
///
/// This centralized list is the source of truth used by the benchmark runner.
/// New `AudioEffect` variants should be added here so they are automatically
/// included in CLI performance reports.
pub fn default_effect_benchmark_cases() -> Vec<AudioEffect> {
    let mut effects = vec![
        AudioEffect::DelayReverb(DelayReverbEffect::default()),
        AudioEffect::DiffusionReverb(DiffusionReverbEffect::default()),
        AudioEffect::ConvolutionReverb(ConvolutionReverbEffect::default()),
        AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
        AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
        AudioEffect::Distortion(DistortionEffect::default()),
        AudioEffect::Gain(GainEffect::default()),
        AudioEffect::Compressor(CompressorEffect::default()),
        AudioEffect::Limiter(LimiterEffect::default()),
        AudioEffect::MultibandEq(MultibandEqEffect::default()),
        AudioEffect::Pan(PanEffect::default()),
    ];
    for effect in &mut effects {
        enable_effect(effect);
    }
    effects
}

/// Default derives for a few settings types leave their `enabled` flag false.
/// A benchmark for an effect must exercise its processing path, so benchmark
/// fixtures always turn the selected effect on.
fn enable_effect(effect: &mut AudioEffect) {
    match effect {
        AudioEffect::DelayReverb(effect) => effect.enabled = true,
        AudioEffect::DiffusionReverb(effect) => effect.enabled = true,
        AudioEffect::ConvolutionReverb(effect) => effect.enabled = true,
        AudioEffect::LowPassFilter(effect) => effect.enabled = true,
        AudioEffect::HighPassFilter(effect) => effect.enabled = true,
        AudioEffect::Distortion(effect) => effect.enabled = true,
        AudioEffect::Gain(effect) => effect.enabled = true,
        AudioEffect::Compressor(effect) => effect.enabled = true,
        AudioEffect::Limiter(effect) => effect.enabled = true,
        AudioEffect::MultibandEq(effect) => effect.enabled = true,
        AudioEffect::Pan(effect) => effect.enabled = true,
    }
}

/// Benchmark no processing, every available default effect, and an optional project chain.
///
/// Setup, cloning, and warm-up are excluded from recorded durations. Effects are
/// reset for every run so stateful effects are measured against the same input state.
pub fn benchmark_audio(
    samples: &[f32],
    config: AudioBenchmarkConfig,
    container_path: Option<String>,
    project_impulse_response: Option<crate::dsp::effects::convolution_reverb::ImpulseResponseSpec>,
    impulse_response_tail_db: f32,
    project_effects: Option<Vec<AudioEffect>>,
) -> Result<AudioBenchmarkReport, AudioBenchmarkError> {
    benchmark_audio_with_progress(
        samples,
        config,
        container_path,
        project_impulse_response,
        impulse_response_tail_db,
        project_effects,
        |_, _, _| {},
    )
}

/// Benchmark audio while reporting each case just before it begins.
///
/// The callback runs outside the timed region and receives a one-based case
/// index, the total number of cases, and the display name of the case.
pub fn benchmark_audio_with_progress<F>(
    samples: &[f32],
    config: AudioBenchmarkConfig,
    container_path: Option<String>,
    project_impulse_response: Option<crate::dsp::effects::convolution_reverb::ImpulseResponseSpec>,
    impulse_response_tail_db: f32,
    project_effects: Option<Vec<AudioEffect>>,
    mut on_case_started: F,
) -> Result<AudioBenchmarkReport, AudioBenchmarkError>
where
    F: FnMut(usize, usize, &str),
{
    if config.chunk_frames == 0 {
        return Err(AudioBenchmarkError::ZeroChunkFrames);
    }
    let has_project_impulse_response = project_impulse_response.is_some();
    let context = EffectContext::new(
        config.sample_rate,
        config.channels,
        container_path,
        project_impulse_response,
        impulse_response_tail_db,
    )
    .map_err(AudioBenchmarkError::InvalidContext)?;
    if samples.len() % config.channels != 0 {
        return Err(AudioBenchmarkError::PartialFrame {
            samples: samples.len(),
            channels: config.channels,
        });
    }

    let mut definitions = vec![("No effects".to_owned(), Vec::new())];
    definitions.extend(default_effect_benchmark_cases().into_iter().map(|effect| {
        let name = if matches!(effect, AudioEffect::ConvolutionReverb(_))
            && !has_project_impulse_response
        {
            "ConvolutionReverb (no IR; pass-through)".to_owned()
        } else {
            effect.display_name().to_owned()
        };
        (name, vec![effect])
    }));
    if let Some(effects) = project_effects {
        definitions.push(("Project chain".to_owned(), effects));
    }

    let total_cases = definitions.len();
    let mut cases = Vec::with_capacity(total_cases);
    for (index, (name, effects)) in definitions.into_iter().enumerate() {
        on_case_started(index + 1, total_cases, &name);
        cases.push(AudioBenchmarkCase {
            effect_count: effects.len(),
            timing: benchmark_case(samples, &context, config, &effects),
            name,
        });
    }
    let frames = samples.len() / config.channels;
    Ok(AudioBenchmarkReport {
        config,
        samples: samples.len(),
        frames,
        audio_duration: Duration::from_secs_f64(frames as f64 / config.sample_rate as f64),
        cases,
    })
}

fn benchmark_case(
    samples: &[f32],
    context: &EffectContext,
    config: AudioBenchmarkConfig,
    effects: &[AudioEffect],
) -> BenchmarkTiming {
    // Initialize costly immutable resources (most notably convolution IRs) once
    // outside the measured section. Each trial receives a clone of that prepared
    // chain, preserving identical initial DSP state without timing project I/O.
    let mut prepared_effects = effects.to_vec();
    for effect in &mut prepared_effects {
        effect.warm_up(context);
    }
    for _ in 0..config.warmup_iterations {
        black_box(run_once(
            samples,
            context,
            config.chunk_frames,
            prepared_effects.clone(),
        ));
    }

    let iterations = config.iterations.max(1);
    let mut total = Duration::ZERO;
    let mut minimum = Duration::MAX;
    let mut maximum = Duration::ZERO;
    for _ in 0..iterations {
        let effects = prepared_effects.clone();
        let started = Instant::now();
        black_box(run_once(samples, context, config.chunk_frames, effects));
        let elapsed = started.elapsed();
        total += elapsed;
        minimum = minimum.min(elapsed);
        maximum = maximum.max(elapsed);
    }
    let average = total / iterations as u32;
    let audio_seconds =
        samples.len() as f64 / context.channels() as f64 / context.sample_rate() as f64;
    BenchmarkTiming {
        total,
        average,
        minimum,
        maximum,
        real_time_factor: if audio_seconds == 0.0 {
            0.0
        } else {
            average.as_secs_f64() / audio_seconds
        },
    }
}

fn run_once(
    samples: &[f32],
    context: &EffectContext,
    chunk_frames: usize,
    mut effects: Vec<AudioEffect>,
) -> usize {
    let chunk_samples = chunk_frames * context.channels();
    let mut current = Vec::with_capacity(chunk_samples);
    let mut next = Vec::with_capacity(chunk_samples);
    let mut output_samples = 0;
    for chunk in samples.chunks(chunk_samples) {
        current.clear();
        current.extend_from_slice(chunk);
        for effect in &mut effects {
            next.clear();
            effect.process_into(&current, &mut next, context, false);
            std::mem::swap(&mut current, &mut next);
        }
        output_samples += current.len();
    }
    // Drain potentially tail-producing effects once, mirroring end-of-stream processing.
    if !effects.is_empty() {
        current.clear();
        for effect in &mut effects {
            next.clear();
            effect.process_into(&current, &mut next, context, true);
            std::mem::swap(&mut current, &mut next);
        }
        output_samples += current.len();
    }
    output_samples
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cases_cover_each_audio_effect_variant() {
        let names: Vec<_> = default_effect_benchmark_cases()
            .iter()
            .map(AudioEffect::display_name)
            .collect();
        assert_eq!(names.len(), 11);
        assert!(names.contains(&"Gain"));
        assert!(names.contains(&"ConvolutionReverb"));
        assert!(names.contains(&"Pan"));
    }

    #[test]
    fn report_includes_default_and_project_cases() {
        let report = benchmark_audio(
            &[0.0, 0.1, -0.1, 0.0],
            AudioBenchmarkConfig {
                iterations: 1,
                warmup_iterations: 0,
                chunk_frames: 1,
                ..Default::default()
            },
            None,
            None,
            -60.0,
            Some(vec![AudioEffect::Gain(GainEffect::default())]),
        )
        .expect("valid benchmark input");
        assert_eq!(report.cases.len(), 13);
        assert_eq!(report.cases[0].name, "No effects");
        assert_eq!(report.cases.last().unwrap().name, "Project chain");
        assert_eq!(report.cases.last().unwrap().effect_count, 1);
        let markdown = report.to_markdown("fixture.wav");
        assert!(markdown.contains("# Proteus audio benchmark"));
        assert!(markdown.contains("Project chain"));
    }

    #[test]
    fn invalid_input_is_rejected_before_running_effects() {
        let error = benchmark_audio(
            &[0.0, 0.1, -0.1],
            AudioBenchmarkConfig::default(),
            None,
            None,
            -60.0,
            None,
        )
        .expect_err("partial stereo frame must fail");
        assert!(matches!(
            error,
            AudioBenchmarkError::PartialFrame {
                samples: 3,
                channels: 2
            }
        ));
    }
}
