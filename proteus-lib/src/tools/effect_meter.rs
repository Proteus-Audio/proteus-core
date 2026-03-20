//! Offline effect-metering helper for CLI diagnostics and tests.

use serde::{Deserialize, Serialize};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatReader, Packet};
use symphonia::core::units::TimeBase;

use crate::dsp::effects::{AudioEffect, EffectContext};
use crate::dsp::guardrails::sanitize_channels;
use crate::dsp::meter::level::measure_peak_rms;
use crate::dsp::meter::{EffectBandSnapshot, EffectLevelSnapshot};
use crate::tools::decode::{get_decoder, get_reader, DecoderOpenError};

/// Summary policy used when condensing per-chunk meter snapshots into one report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectMeterSummaryMode {
    /// Keep the final non-empty snapshot observed during the run.
    Final,
    /// Keep the maximum observed per-channel values across the run.
    Max,
}

/// Input configuration for the offline effect-meter report builder.
#[derive(Debug, Clone)]
pub struct EffectMeterRunConfig {
    /// Media file to decode.
    pub input_path: String,
    /// Ordered effect chain to apply offline.
    pub effects: Vec<AudioEffect>,
    /// Optional start offset in seconds.
    pub seek_seconds: f64,
    /// Optional capture duration in seconds.
    pub duration_seconds: Option<f64>,
    /// Snapshot reduction mode.
    pub summary_mode: EffectMeterSummaryMode,
    /// Include spectral buckets for supported filter effects.
    pub include_spectral: bool,
}

/// Deterministic offline effect-metering report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectMeterReport {
    /// Input file that was inspected.
    pub input_path: String,
    /// Audio sample rate in Hz.
    pub sample_rate: u32,
    /// Interleaved channel count.
    pub channels: usize,
    /// Number of frames processed after seek/duration trimming.
    pub frames_processed: usize,
    /// Summary mode used for the report.
    pub summary_mode: EffectMeterSummaryMode,
    /// Per-effect level summaries.
    pub effects: Vec<EffectMeterRow>,
    /// Optional per-effect spectral summaries aligned to `effects`.
    pub spectral: Option<Vec<Option<EffectBandSnapshot>>>,
}

/// Per-effect time-domain report row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectMeterRow {
    /// Zero-based effect index.
    pub effect_index: usize,
    /// Human-readable effect name.
    pub effect_name: String,
    /// Time-domain before/after levels for this effect slot.
    pub levels: EffectLevelSnapshot,
}

/// Errors produced while building an offline effect-meter report.
#[derive(Debug)]
pub enum EffectMeterError {
    /// The input media could not be opened.
    Open(DecoderOpenError),
    /// The input format exposed no supported audio track.
    NoSupportedAudioTrack,
    /// A decode error occurred while reading the audio stream.
    Decode(SymphoniaError),
    /// No decodable audio samples were processed after trimming.
    NoDecodedAudio,
    /// Directory inputs are not supported by the offline meter helper.
    UnsupportedDirectoryInput(String),
    /// One decoded packet contradicted the earlier stream format.
    StreamFormatChanged {
        expected_sample_rate: u32,
        expected_channels: usize,
        actual_sample_rate: u32,
        actual_channels: usize,
    },
    /// The helper could not construct a valid effect-processing context.
    InvalidEffectContext(String),
}

impl std::fmt::Display for EffectMeterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(err) => write!(f, "failed to open input: {}", err),
            Self::NoSupportedAudioTrack => write!(f, "no supported audio track found"),
            Self::Decode(err) => write!(f, "failed to decode audio: {}", err),
            Self::NoDecodedAudio => write!(f, "no decodable audio remained after trimming"),
            Self::UnsupportedDirectoryInput(path) => write!(
                f,
                "directory inputs are not supported by the offline effect meter: {}",
                path
            ),
            Self::StreamFormatChanged {
                expected_sample_rate,
                expected_channels,
                actual_sample_rate,
                actual_channels,
            } => write!(
                f,
                "stream format changed during decode (expected {} Hz / {} ch, got {} Hz / {} ch)",
                expected_sample_rate, expected_channels, actual_sample_rate, actual_channels
            ),
            Self::InvalidEffectContext(err) => write!(f, "invalid effect context: {}", err),
        }
    }
}

impl std::error::Error for EffectMeterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open(err) => Some(err),
            Self::Decode(err) => Some(err),
            _ => None,
        }
    }
}

impl From<DecoderOpenError> for EffectMeterError {
    fn from(value: DecoderOpenError) -> Self {
        Self::Open(value)
    }
}

/// Build a deterministic effect-meter report by decoding and processing `config.input_path`.
pub fn run_report(config: &EffectMeterRunConfig) -> Result<EffectMeterReport, EffectMeterError> {
    if std::path::Path::new(&config.input_path).is_dir() {
        return Err(EffectMeterError::UnsupportedDirectoryInput(
            config.input_path.clone(),
        ));
    }

    let mut format = get_reader(&config.input_path)?;
    let track = find_audio_track(format.as_ref())?;
    let track_id = track.id;
    let packet_time_base = track.codec_params.time_base;
    let packet_sample_rate = track.codec_params.sample_rate;
    let mut decoder = get_decoder(format.as_ref())?;
    let mut runtime = OfflineMeterRuntime::new(config);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => return Err(EffectMeterError::Decode(err)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => {
                continue;
            }
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => return Err(EffectMeterError::Decode(err)),
        };

        let packet_seconds = packet_ts_seconds(packet.ts(), packet_time_base, packet_sample_rate);
        let continue_processing = runtime.process_packet(packet_seconds, &packet, decoded)?;
        if !continue_processing {
            break;
        }
    }

    runtime.finish()
}

fn find_audio_track(
    format: &dyn FormatReader,
) -> Result<&symphonia::core::formats::Track, EffectMeterError> {
    format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(EffectMeterError::NoSupportedAudioTrack)
}

fn packet_ts_seconds(ts: u64, time_base: Option<TimeBase>, sample_rate: Option<u32>) -> f64 {
    if let Some(base) = time_base {
        let time = base.calc_time(ts);
        return time.seconds as f64 + time.frac;
    }
    if let Some(rate) = sample_rate {
        return ts as f64 / rate.max(1) as f64;
    }
    0.0
}

struct OfflineMeterRuntime<'a> {
    config: &'a EffectMeterRunConfig,
    effects: Vec<AudioEffect>,
    state: Option<OfflineAudioState>,
}

struct OfflineAudioState {
    input_path: String,
    sample_rate: u32,
    channels: usize,
    context: EffectContext,
    frames_processed: usize,
    scratch_a: Vec<f32>,
    scratch_b: Vec<f32>,
    levels: Vec<EffectLevelSnapshot>,
    #[cfg(feature = "effect-meter-spectral")]
    analyzers: Vec<Option<crate::dsp::meter::spectral::EffectSpectralAnalyzer>>,
}

impl<'a> OfflineMeterRuntime<'a> {
    fn new(config: &'a EffectMeterRunConfig) -> Self {
        Self {
            config,
            effects: config.effects.clone(),
            state: None,
        }
    }

    fn process_packet(
        &mut self,
        packet_seconds: f64,
        _packet: &Packet,
        decoded: AudioBufferRef<'_>,
    ) -> Result<bool, EffectMeterError> {
        let channels = sanitize_channels(decoded.spec().channels.count());
        let sample_rate = decoded.spec().rate.max(1);
        let mut samples = interleaved_samples(decoded, channels);
        if samples.is_empty() {
            return Ok(true);
        }

        self.ensure_state(sample_rate, channels)?;
        let state = self.state.as_mut().expect("state initialized");
        let frames = samples.len() / channels.max(1);
        let packet_end = packet_seconds + frames as f64 / sample_rate as f64;
        let capture_start = self.config.seek_seconds.max(0.0);
        let capture_end = self
            .config
            .duration_seconds
            .map(|seconds| capture_start + seconds.max(0.0));

        if packet_end <= capture_start {
            return Ok(true);
        }
        if let Some(end) = capture_end {
            if packet_seconds >= end {
                return Ok(false);
            }
        }

        trim_packet_samples(
            &mut samples,
            channels,
            sample_rate,
            packet_seconds,
            capture_start,
            capture_end,
        );
        if samples.is_empty() {
            return Ok(true);
        }

        state.frames_processed = state
            .frames_processed
            .saturating_add(samples.len() / channels.max(1));
        process_effect_chunk(
            &mut self.effects,
            &samples,
            &state.context,
            false,
            &mut state.scratch_a,
            &mut state.scratch_b,
            &mut state.levels,
            #[cfg(feature = "effect-meter-spectral")]
            &mut state.analyzers,
            self.config.summary_mode,
        );

        Ok(capture_end.map(|end| packet_end < end).unwrap_or(true))
    }

    fn ensure_state(&mut self, sample_rate: u32, channels: usize) -> Result<(), EffectMeterError> {
        if let Some(state) = self.state.as_ref() {
            if state.sample_rate != sample_rate || state.channels != channels {
                return Err(EffectMeterError::StreamFormatChanged {
                    expected_sample_rate: state.sample_rate,
                    expected_channels: state.channels,
                    actual_sample_rate: sample_rate,
                    actual_channels: channels,
                });
            }
            return Ok(());
        }

        let context = EffectContext::new(
            sample_rate,
            channels,
            Some(self.config.input_path.clone()),
            None,
            -60.0,
        )
        .map_err(|err| EffectMeterError::InvalidEffectContext(err.to_string()))?;
        for effect in &mut self.effects {
            effect.warm_up(&context);
        }

        self.state = Some(OfflineAudioState {
            input_path: self.config.input_path.clone(),
            sample_rate,
            channels,
            context,
            frames_processed: 0,
            scratch_a: Vec::new(),
            scratch_b: Vec::new(),
            levels: zeroed_snapshots(self.effects.len(), channels),
            #[cfg(feature = "effect-meter-spectral")]
            analyzers: build_spectral_analyzers(
                self.config.include_spectral,
                &self.effects,
                channels,
            ),
        });

        Ok(())
    }

    fn finish(mut self) -> Result<EffectMeterReport, EffectMeterError> {
        let Some(mut state) = self.state.take() else {
            return Err(EffectMeterError::NoDecodedAudio);
        };

        if state.frames_processed == 0 {
            return Err(EffectMeterError::NoDecodedAudio);
        }

        let mut drain_levels = zeroed_snapshots(self.effects.len(), state.channels);
        process_effect_chunk(
            &mut self.effects,
            &[],
            &state.context,
            true,
            &mut state.scratch_a,
            &mut state.scratch_b,
            &mut drain_levels,
            #[cfg(feature = "effect-meter-spectral")]
            &mut state.analyzers,
            self.config.summary_mode,
        );
        if !state.scratch_a.is_empty() {
            state.levels = merge_summary(&state.levels, &drain_levels, self.config.summary_mode);
        }

        let effects = self
            .effects
            .iter()
            .enumerate()
            .map(|(index, effect)| EffectMeterRow {
                effect_index: index,
                effect_name: effect.display_name().to_string(),
                levels: state.levels[index].clone(),
            })
            .collect();

        #[cfg(feature = "effect-meter-spectral")]
        let spectral = if self.config.include_spectral {
            Some(
                state
                    .analyzers
                    .iter_mut()
                    .enumerate()
                    .map(|(index, analyzer)| {
                        analyzer.as_mut().and_then(|analyzer| {
                            analyzer.analyze(&self.effects[index], state.sample_rate)
                        })
                    })
                    .collect(),
            )
        } else {
            None
        };

        #[cfg(not(feature = "effect-meter-spectral"))]
        let spectral = None;

        Ok(EffectMeterReport {
            input_path: state.input_path,
            sample_rate: state.sample_rate,
            channels: state.channels,
            frames_processed: state.frames_processed,
            summary_mode: self.config.summary_mode,
            effects,
            spectral,
        })
    }
}

fn interleaved_samples(decoded: AudioBufferRef<'_>, channels: usize) -> Vec<f32> {
    let channels = sanitize_channels(channels);
    let mut per_channel = Vec::with_capacity(channels);
    for channel in 0..channels {
        per_channel.push(crate::audio::decode::process_channel(
            decoded.clone(),
            channel,
        ));
    }
    let frames = per_channel.first().map(Vec::len).unwrap_or(0);
    let mut interleaved = Vec::with_capacity(frames.saturating_mul(channels));
    for frame_index in 0..frames {
        for channel in 0..channels {
            interleaved.push(per_channel[channel][frame_index]);
        }
    }
    interleaved
}

fn trim_packet_samples(
    samples: &mut Vec<f32>,
    channels: usize,
    sample_rate: u32,
    packet_seconds: f64,
    capture_start: f64,
    capture_end: Option<f64>,
) {
    let channels = channels.max(1);
    let frame_count = samples.len() / channels;
    if frame_count == 0 {
        samples.clear();
        return;
    }

    let mut start_frame = 0_usize;
    if packet_seconds < capture_start {
        start_frame = ((capture_start - packet_seconds) * sample_rate as f64).floor() as usize;
    }

    let mut end_frame = frame_count;
    if let Some(end_seconds) = capture_end {
        let packet_end = packet_seconds + frame_count as f64 / sample_rate as f64;
        if packet_end > end_seconds {
            let trailing = ((packet_end - end_seconds) * sample_rate as f64)
                .ceil()
                .max(0.0) as usize;
            end_frame = end_frame.saturating_sub(trailing);
        }
    }

    start_frame = start_frame.min(frame_count);
    end_frame = end_frame.clamp(start_frame, frame_count);
    let start_index = start_frame.saturating_mul(channels);
    let end_index = end_frame.saturating_mul(channels);
    if start_index == 0 && end_index == samples.len() {
        return;
    }
    let trimmed = samples[start_index..end_index].to_vec();
    samples.clear();
    samples.extend_from_slice(&trimmed);
}

fn process_effect_chunk(
    effects: &mut [AudioEffect],
    input: &[f32],
    context: &EffectContext,
    drain: bool,
    scratch_a: &mut Vec<f32>,
    scratch_b: &mut Vec<f32>,
    summary_levels: &mut Vec<EffectLevelSnapshot>,
    #[cfg(feature = "effect-meter-spectral")] analyzers: &mut [Option<
        crate::dsp::meter::spectral::EffectSpectralAnalyzer,
    >],
    summary_mode: EffectMeterSummaryMode,
) {
    scratch_a.clear();
    scratch_a.extend_from_slice(input);

    if summary_levels.len() != effects.len() {
        *summary_levels = zeroed_snapshots(effects.len(), context.channels());
    }
    let mut chunk_levels = zeroed_snapshots(effects.len(), context.channels());
    let channels = context.channels().max(1);

    for (index, effect) in effects.iter_mut().enumerate() {
        chunk_levels[index].input = measure_peak_rms(scratch_a, channels);
        #[cfg(feature = "effect-meter-spectral")]
        if let Some(analyzer) = analyzers.get_mut(index).and_then(Option::as_mut) {
            analyzer.capture_input(scratch_a);
        }

        scratch_b.clear();
        effect.process_into(scratch_a, scratch_b, context, drain);
        std::mem::swap(scratch_a, scratch_b);

        chunk_levels[index].output = measure_peak_rms(scratch_a, channels);
        #[cfg(feature = "effect-meter-spectral")]
        if let Some(analyzer) = analyzers.get_mut(index).and_then(Option::as_mut) {
            analyzer.capture_output(scratch_a);
        }
    }

    *summary_levels = merge_summary(summary_levels, &chunk_levels, summary_mode);
}

fn merge_summary(
    current: &[EffectLevelSnapshot],
    next: &[EffectLevelSnapshot],
    summary_mode: EffectMeterSummaryMode,
) -> Vec<EffectLevelSnapshot> {
    match summary_mode {
        EffectMeterSummaryMode::Final => next.to_vec(),
        EffectMeterSummaryMode::Max => current
            .iter()
            .zip(next.iter())
            .map(|(current, next)| EffectLevelSnapshot {
                input: merge_level_snapshot_max(&current.input, &next.input),
                output: merge_level_snapshot_max(&current.output, &next.output),
            })
            .collect(),
    }
}

fn merge_level_snapshot_max(
    current: &crate::dsp::meter::LevelSnapshot,
    next: &crate::dsp::meter::LevelSnapshot,
) -> crate::dsp::meter::LevelSnapshot {
    let channels = current.peak.len().max(next.peak.len());
    let mut peak = vec![0.0; channels];
    let mut rms = vec![0.0; channels];
    for index in 0..channels {
        peak[index] = current
            .peak
            .get(index)
            .copied()
            .unwrap_or(0.0)
            .max(next.peak.get(index).copied().unwrap_or(0.0));
        rms[index] = current
            .rms
            .get(index)
            .copied()
            .unwrap_or(0.0)
            .max(next.rms.get(index).copied().unwrap_or(0.0));
    }
    crate::dsp::meter::LevelSnapshot { peak, rms }
}

fn zeroed_snapshots(effect_count: usize, channels: usize) -> Vec<EffectLevelSnapshot> {
    (0..effect_count)
        .map(|_| EffectLevelSnapshot {
            input: crate::dsp::meter::LevelSnapshot {
                peak: vec![0.0; channels],
                rms: vec![0.0; channels],
            },
            output: crate::dsp::meter::LevelSnapshot {
                peak: vec![0.0; channels],
                rms: vec![0.0; channels],
            },
        })
        .collect()
}

#[cfg(feature = "effect-meter-spectral")]
fn build_spectral_analyzers(
    include_spectral: bool,
    effects: &[AudioEffect],
    channels: usize,
) -> Vec<Option<crate::dsp::meter::spectral::EffectSpectralAnalyzer>> {
    if !include_spectral {
        return std::iter::repeat_with(|| None)
            .take(effects.len())
            .collect();
    }
    effects
        .iter()
        .map(|effect| {
            crate::dsp::meter::spectral::relevant_effect(effect)
                .then(|| crate::dsp::meter::spectral::EffectSpectralAnalyzer::new(channels, 2048))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::dsp::effects::{AudioEffect, GainEffect};

    use super::{run_report, EffectMeterRunConfig, EffectMeterSummaryMode};

    fn fixture_path(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test_audio")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn gain_report_shows_higher_output_peak_than_input_peak() {
        let mut gain = GainEffect::default();
        gain.enabled = true;
        gain.settings.gain = 2.0;
        let report = run_report(&EffectMeterRunConfig {
            input_path: fixture_path("test-16bit.wav"),
            effects: vec![AudioEffect::Gain(gain)],
            seek_seconds: 0.0,
            duration_seconds: Some(0.25),
            summary_mode: EffectMeterSummaryMode::Max,
            include_spectral: false,
        })
        .expect("report");

        assert_eq!(report.effects.len(), 1);
        let row = &report.effects[0];
        assert!(row.levels.output.peak[0] > row.levels.input.peak[0]);
        assert!(report.frames_processed > 0);
    }

    #[cfg(feature = "effect-meter-spectral")]
    #[test]
    fn spectral_report_is_populated_for_supported_filter_effects() {
        use crate::dsp::effects::HighPassFilterEffect;

        let report = run_report(&EffectMeterRunConfig {
            input_path: fixture_path("test-16bit.wav"),
            effects: vec![AudioEffect::HighPassFilter(HighPassFilterEffect::default())],
            seek_seconds: 0.0,
            duration_seconds: Some(0.25),
            summary_mode: EffectMeterSummaryMode::Final,
            include_spectral: true,
        })
        .expect("report");

        let spectral = report.spectral.expect("spectral section");
        assert_eq!(spectral.len(), 1);
        assert!(spectral[0].is_some());
    }
}
