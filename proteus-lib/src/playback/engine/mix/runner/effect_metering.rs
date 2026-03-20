#![cfg_attr(
    not(any(feature = "effect-meter", feature = "effect-meter-spectral")),
    allow(dead_code, unused_imports, unused_mut, unused_variables)
)]

//! Mix-thread local state for per-effect metering and spectral analysis.

use std::sync::Arc;

use crate::dsp::effects::{AudioEffect, EffectContext};
use crate::dsp::guardrails::{sanitize_channels, sanitize_sample_rate};
use crate::dsp::meter::level::{measure_peak_rms_into, resize_level_snapshot};
#[cfg(feature = "effect-meter-spectral")]
use crate::dsp::meter::EffectBandSnapshot;
use crate::dsp::meter::EffectLevelSnapshot;
use crate::playback::effect_meter::EffectMeter;

use super::super::effects::EffectChainObserver;

pub(super) struct MixEffectMeteringState {
    shared: Arc<EffectMeter>,
    mix_time_secs: f64,
    #[cfg(feature = "effect-meter")]
    level: LevelMeterRuntime,
    #[cfg(feature = "effect-meter-spectral")]
    spectral: SpectralMeterRuntime,
}

impl MixEffectMeteringState {
    pub(super) fn new(
        shared: Arc<EffectMeter>,
        effects: &[AudioEffect],
        context: &EffectContext,
    ) -> Self {
        let mut state = Self {
            shared,
            mix_time_secs: 0.0,
            #[cfg(feature = "effect-meter")]
            level: LevelMeterRuntime::new(effects.len(), context.channels()),
            #[cfg(feature = "effect-meter-spectral")]
            spectral: SpectralMeterRuntime::new(),
        };
        state.reset_for_chain(effects, context, false);
        state
    }

    /// Set the producer-clock position for the next snapshot.
    ///
    /// Called before `prepare_chunk` so that `finish` can tag the snapshot
    /// with the cumulative mix-time at the mix→worker boundary.
    pub(super) fn set_mix_time(&mut self, secs: f64) {
        self.mix_time_secs = secs;
    }

    pub(super) fn prepare_chunk<'a>(
        &'a mut self,
        effects: &[AudioEffect],
        context: &EffectContext,
        frames: usize,
    ) -> Option<ChunkEffectMetering<'a>> {
        let mut active = false;

        #[cfg(feature = "effect-meter")]
        let level_due = {
            let due = self
                .level
                .prepare(&self.shared, effects.len(), context, frames);
            active |= due;
            due
        };
        #[cfg(not(feature = "effect-meter"))]
        let level_due = false;

        #[cfg(feature = "effect-meter-spectral")]
        let (spectral_active, spectral_due) = {
            let prepared = self
                .spectral
                .prepare(&self.shared, effects, context, frames);
            active |= prepared.active;
            (prepared.active, prepared.due)
        };

        if !active {
            return None;
        }

        Some(ChunkEffectMetering {
            state: self,
            level_due,
            #[cfg(feature = "effect-meter-spectral")]
            spectral_active,
            #[cfg(feature = "effect-meter-spectral")]
            spectral_due,
        })
    }

    pub(super) fn reset_for_chain(
        &mut self,
        effects: &[AudioEffect],
        context: &EffectContext,
        publish_zero: bool,
    ) {
        #[cfg(feature = "effect-meter")]
        self.level
            .reset_for_chain(effects.len(), context.channels());

        #[cfg(feature = "effect-meter-spectral")]
        self.spectral.reset_for_chain(effects, context);

        if publish_zero {
            self.publish_zero_layout();
        }
    }

    pub(super) fn publish_zero_layout(&self) {
        #[cfg(feature = "effect-meter")]
        self.shared.try_publish_levels(&self.level.snapshots);

        #[cfg(feature = "effect-meter-spectral")]
        self.shared.try_publish_spectral(&self.spectral.snapshots);
    }
}

pub(super) struct ChunkEffectMetering<'a> {
    state: &'a mut MixEffectMeteringState,
    level_due: bool,
    #[cfg(feature = "effect-meter-spectral")]
    spectral_active: bool,
    #[cfg(feature = "effect-meter-spectral")]
    spectral_due: bool,
}

impl ChunkEffectMetering<'_> {
    pub(super) fn finish(self, _effects: &[AudioEffect]) {
        #[cfg(feature = "effect-meter")]
        if self.level_due {
            self.state
                .shared
                .try_publish_levels(&self.state.level.snapshots);
            self.state
                .shared
                .push_timestamped_levels(self.state.mix_time_secs, &self.state.level.snapshots);
        }

        #[cfg(feature = "effect-meter-spectral")]
        if self.spectral_due {
            self.state.spectral.publish(&self.state.shared, _effects);
            self.state.shared.push_timestamped_spectral(
                self.state.mix_time_secs,
                &self.state.spectral.snapshots,
            );
        }
    }
}

impl EffectChainObserver for ChunkEffectMetering<'_> {
    fn before_effect(
        &mut self,
        effect_index: usize,
        _effect: &AudioEffect,
        input: &[f32],
        channels: usize,
    ) {
        #[cfg(feature = "effect-meter")]
        if self.level_due {
            if let Some(snapshot) = self.state.level.snapshots.get_mut(effect_index) {
                measure_peak_rms_into(input, channels, &mut snapshot.input);
            }
        }

        #[cfg(feature = "effect-meter-spectral")]
        if self.spectral_active {
            if let Some(analyzer) = self
                .state
                .spectral
                .analyzers
                .get_mut(effect_index)
                .and_then(Option::as_mut)
            {
                analyzer.capture_input(input);
            }
        }
    }

    fn after_effect(
        &mut self,
        effect_index: usize,
        _effect: &AudioEffect,
        output: &[f32],
        channels: usize,
    ) {
        #[cfg(feature = "effect-meter")]
        if self.level_due {
            if let Some(snapshot) = self.state.level.snapshots.get_mut(effect_index) {
                measure_peak_rms_into(output, channels, &mut snapshot.output);
            }
        }

        #[cfg(feature = "effect-meter-spectral")]
        if self.spectral_active {
            if let Some(analyzer) = self
                .state
                .spectral
                .analyzers
                .get_mut(effect_index)
                .and_then(Option::as_mut)
            {
                analyzer.capture_output(output);
            }
        }
    }
}

#[cfg(feature = "effect-meter")]
struct LevelMeterRuntime {
    sample_rate: u32,
    channels: usize,
    refresh_hz: f32,
    frame_samples_per_channel: usize,
    accumulated_frames: usize,
    enabled: bool,
    snapshots: Vec<EffectLevelSnapshot>,
}

#[cfg(feature = "effect-meter")]
impl LevelMeterRuntime {
    fn new(effect_count: usize, channels: usize) -> Self {
        let mut runtime = Self {
            sample_rate: 48_000,
            channels: sanitize_channels(channels),
            refresh_hz: 30.0,
            frame_samples_per_channel: 1,
            accumulated_frames: 0,
            enabled: false,
            snapshots: Vec::new(),
        };
        runtime.reset_for_chain(effect_count, channels);
        runtime
    }

    fn prepare(
        &mut self,
        shared: &EffectMeter,
        effect_count: usize,
        context: &EffectContext,
        frames: usize,
    ) -> bool {
        self.sample_rate = sanitize_sample_rate(context.sample_rate());
        let refresh_hz = shared.level_refresh_hz();
        if (self.refresh_hz - refresh_hz).abs() > f32::EPSILON {
            self.refresh_hz = refresh_hz;
            self.frame_samples_per_channel =
                frame_samples_per_channel(self.sample_rate, self.refresh_hz);
            self.accumulated_frames = 0;
        }

        self.reset_for_chain(effect_count, context.channels());

        let enabled = shared.level_metering_enabled();
        if !enabled {
            self.enabled = false;
            self.accumulated_frames = 0;
            return false;
        }
        if !self.enabled {
            self.enabled = true;
            self.accumulated_frames = 0;
        }

        self.accumulated_frames = self.accumulated_frames.saturating_add(frames);
        if self.accumulated_frames >= self.frame_samples_per_channel {
            self.accumulated_frames %= self.frame_samples_per_channel;
            true
        } else {
            false
        }
    }

    fn reset_for_chain(&mut self, effect_count: usize, channels: usize) {
        let channels = sanitize_channels(channels);
        let layout_changed = self.channels != channels || self.snapshots.len() != effect_count;
        if !layout_changed {
            return;
        }

        self.channels = channels;
        self.accumulated_frames = 0;
        self.snapshots
            .resize_with(effect_count, EffectLevelSnapshot::default);
        for snapshot in self.snapshots.iter_mut() {
            resize_level_snapshot(&mut snapshot.input, channels);
            resize_level_snapshot(&mut snapshot.output, channels);
            snapshot.input.peak.fill(0.0);
            snapshot.input.rms.fill(0.0);
            snapshot.output.peak.fill(0.0);
            snapshot.output.rms.fill(0.0);
        }
    }
}

#[cfg(feature = "effect-meter-spectral")]
struct SpectralPrepare {
    active: bool,
    due: bool,
}

#[cfg(feature = "effect-meter-spectral")]
struct SpectralMeterRuntime {
    sample_rate: u32,
    channels: usize,
    refresh_hz: f32,
    frame_samples_per_channel: usize,
    accumulated_frames: usize,
    analyzers: Vec<Option<crate::dsp::meter::spectral::EffectSpectralAnalyzer>>,
    snapshots: Vec<Option<EffectBandSnapshot>>,
}

#[cfg(feature = "effect-meter-spectral")]
impl SpectralMeterRuntime {
    fn new() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            refresh_hz: 15.0,
            frame_samples_per_channel: 1,
            accumulated_frames: 0,
            analyzers: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    fn prepare(
        &mut self,
        shared: &EffectMeter,
        effects: &[AudioEffect],
        context: &EffectContext,
        frames: usize,
    ) -> SpectralPrepare {
        if !shared.spectral_analysis_enabled() {
            self.accumulated_frames = 0;
            return SpectralPrepare {
                active: false,
                due: false,
            };
        }

        let sample_rate = sanitize_sample_rate(context.sample_rate());
        let channels = sanitize_channels(context.channels());
        let refresh_hz = shared.spectral_refresh_hz();
        if self.sample_rate != sample_rate
            || self.channels != channels
            || (self.refresh_hz - refresh_hz).abs() > f32::EPSILON
            || self.analyzers.len() != effects.len()
        {
            self.rebuild(effects, sample_rate, channels, refresh_hz);
            self.accumulated_frames = 0;
        }

        self.accumulated_frames = self.accumulated_frames.saturating_add(frames);
        let due = self.accumulated_frames >= self.frame_samples_per_channel;
        if due {
            self.accumulated_frames %= self.frame_samples_per_channel;
        }

        SpectralPrepare { active: true, due }
    }

    fn reset_for_chain(&mut self, effects: &[AudioEffect], context: &EffectContext) {
        self.rebuild(
            effects,
            sanitize_sample_rate(context.sample_rate()),
            sanitize_channels(context.channels()),
            self.refresh_hz,
        );
        self.accumulated_frames = 0;
    }

    fn publish(&mut self, shared: &EffectMeter, effects: &[AudioEffect]) {
        self.snapshots.resize(effects.len(), None);
        for (index, (effect, analyzer)) in effects.iter().zip(self.analyzers.iter_mut()).enumerate()
        {
            self.snapshots[index] = analyzer
                .as_mut()
                .and_then(|analyzer| analyzer.analyze(effect, self.sample_rate));
        }
        shared.try_publish_spectral(&self.snapshots);
    }

    fn rebuild(
        &mut self,
        effects: &[AudioEffect],
        sample_rate: u32,
        channels: usize,
        refresh_hz: f32,
    ) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.refresh_hz = refresh_hz;
        self.frame_samples_per_channel = frame_samples_per_channel(sample_rate, refresh_hz);
        let fft_frames = self.frame_samples_per_channel.next_power_of_two().max(1);
        self.analyzers = effects
            .iter()
            .map(|effect| {
                crate::dsp::meter::spectral::relevant_effect(effect).then(|| {
                    crate::dsp::meter::spectral::EffectSpectralAnalyzer::new(channels, fft_frames)
                })
            })
            .collect();
        self.snapshots.clear();
        self.snapshots.resize(effects.len(), None);
    }
}

fn frame_samples_per_channel(sample_rate: u32, refresh_hz: f32) -> usize {
    ((sanitize_sample_rate(sample_rate) as f32 / refresh_hz.max(1.0)).round() as usize).max(1)
}
