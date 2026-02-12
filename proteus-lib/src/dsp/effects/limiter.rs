//! Limiter effect using rodio's built-in limiter.

use std::collections::VecDeque;
use std::time::Duration;

use rodio::source::{Limit, LimitSettings, SeekError, Source};
use serde::{Deserialize, Serialize};

use super::level::deserialize_db_gain;
use super::EffectContext;

const DEFAULT_THRESHOLD_DB: f32 = -1.0;
const DEFAULT_KNEE_WIDTH_DB: f32 = 4.0;
const DEFAULT_ATTACK_MS: f32 = 5.0;
const DEFAULT_RELEASE_MS: f32 = 100.0;

/// Serialized configuration for limiter parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimiterSettings {
    #[serde(
        alias = "threshold",
        alias = "threshold_db",
        deserialize_with = "deserialize_db_gain"
    )]
    pub threshold_db: f32,
    #[serde(alias = "knee_width", alias = "knee_width_db")]
    pub knee_width_db: f32,
    #[serde(alias = "attack_ms", alias = "attack")]
    pub attack_ms: f32,
    #[serde(alias = "release_ms", alias = "release")]
    pub release_ms: f32,
}

impl LimiterSettings {
    /// Create limiter settings.
    pub fn new(threshold_db: f32, knee_width_db: f32, attack_ms: f32, release_ms: f32) -> Self {
        Self {
            threshold_db,
            knee_width_db,
            attack_ms,
            release_ms,
        }
    }
}

impl Default for LimiterSettings {
    fn default() -> Self {
        Self {
            threshold_db: DEFAULT_THRESHOLD_DB,
            knee_width_db: DEFAULT_KNEE_WIDTH_DB,
            attack_ms: DEFAULT_ATTACK_MS,
            release_ms: DEFAULT_RELEASE_MS,
        }
    }
}

/// Configured limiter effect with runtime state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimiterEffect {
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: LimiterSettings,
    #[serde(skip)]
    state: Option<LimiterState>,
}

impl std::fmt::Debug for LimiterEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LimiterEffect")
            .field("enabled", &self.enabled)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Default for LimiterEffect {
    fn default() -> Self {
        Self {
            enabled: false,
            settings: LimiterSettings::default(),
            state: None,
        }
    }
}

impl LimiterEffect {
    /// Process interleaved samples through the limiter.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: Unused for this effect.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, _drain: bool) -> Vec<f32> {
        if !self.enabled {
            return samples.to_vec();
        }

        self.ensure_state(context);
        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        if samples.is_empty() {
            return Vec::new();
        }

        state.process(samples)
    }

    /// Reset any internal state held by the limiter.
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let settings = sanitize_settings(&self.settings);
        let channels = context.channels.max(1);

        let needs_reset = self
            .state
            .as_ref()
            .map(|state| !state.matches(context.sample_rate, channels, &settings))
            .unwrap_or(true);

        if needs_reset {
            self.state = Some(LimiterState::new(context.sample_rate, channels, settings));
        }
    }
}

#[derive(Clone)]
struct LimiterState {
    sample_rate: u32,
    channels: usize,
    settings: LimiterSettings,
    limiter: Limit<ChunkSource>,
}

impl LimiterState {
    fn new(sample_rate: u32, channels: usize, settings: LimiterSettings) -> Self {
        let source = ChunkSource::new(channels as u16, sample_rate);
        let limiter = source.limit(build_limit_settings(&settings));
        Self {
            sample_rate,
            channels,
            settings,
            limiter,
        }
    }

    fn matches(&self, sample_rate: u32, channels: usize, settings: &LimiterSettings) -> bool {
        self.sample_rate == sample_rate
            && self.channels == channels
            && (self.settings.threshold_db - settings.threshold_db).abs() < f32::EPSILON
            && (self.settings.knee_width_db - settings.knee_width_db).abs() < f32::EPSILON
            && (self.settings.attack_ms - settings.attack_ms).abs() < f32::EPSILON
            && (self.settings.release_ms - settings.release_ms).abs() < f32::EPSILON
    }

    fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        {
            let inner = self.limiter.inner_mut();
            inner.push_samples(samples);
        }

        let mut output = Vec::with_capacity(samples.len());
        for _ in 0..samples.len() {
            if let Some(sample) = self.limiter.next() {
                output.push(sample);
            } else {
                break;
            }
        }
        output
    }

    fn reset(&mut self) {
        let source = ChunkSource::new(self.channels as u16, self.sample_rate);
        self.limiter = source.limit(build_limit_settings(&self.settings));
    }
}

#[derive(Clone, Debug)]
struct ChunkSource {
    channels: u16,
    sample_rate: u32,
    queue: VecDeque<f32>,
}

impl ChunkSource {
    fn new(channels: u16, sample_rate: u32) -> Self {
        Self {
            channels,
            sample_rate,
            queue: VecDeque::new(),
        }
    }

    fn push_samples(&mut self, samples: &[f32]) {
        self.queue.extend(samples.iter().copied());
    }
}

impl Iterator for ChunkSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.queue.pop_front()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.queue.len();
        (len, Some(len))
    }
}

impl Source for ChunkSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.queue.len())
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, _pos: Duration) -> Result<(), SeekError> {
        Err(SeekError::NotSupported {
            underlying_source: "ChunkSource",
        })
    }
}

fn build_limit_settings(settings: &LimiterSettings) -> LimitSettings {
    LimitSettings::default()
        .with_threshold(settings.threshold_db)
        .with_knee_width(settings.knee_width_db)
        .with_attack(Duration::from_secs_f32(settings.attack_ms / 1000.0))
        .with_release(Duration::from_secs_f32(settings.release_ms / 1000.0))
}

fn sanitize_settings(settings: &LimiterSettings) -> LimiterSettings {
    LimiterSettings {
        threshold_db: sanitize_threshold_db(settings.threshold_db),
        knee_width_db: sanitize_knee_width_db(settings.knee_width_db),
        attack_ms: sanitize_time_ms(settings.attack_ms, DEFAULT_ATTACK_MS),
        release_ms: sanitize_time_ms(settings.release_ms, DEFAULT_RELEASE_MS),
    }
}

fn sanitize_threshold_db(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_THRESHOLD_DB;
    }
    value.min(0.0)
}

fn sanitize_knee_width_db(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_KNEE_WIDTH_DB;
    }
    value.max(0.1)
}

fn sanitize_time_ms(value: f32, fallback: f32) -> f32 {
    if !value.is_finite() {
        return fallback;
    }
    value.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(channels: usize) -> EffectContext {
        EffectContext {
            sample_rate: 48_000,
            channels,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        }
    }

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn limiter_disabled_passthrough() {
        let mut effect = LimiterEffect::default();
        let samples = vec![0.25_f32, -0.25, 0.5, -0.5];
        let output = effect.process(&samples, &context(2), false);
        assert_eq!(output, samples);
    }

    #[test]
    fn limiter_reduces_hot_signal() {
        let mut effect = LimiterEffect::default();
        effect.enabled = true;
        effect.settings.threshold_db = -12.0;
        effect.settings.knee_width_db = 0.5;
        effect.settings.attack_ms = 0.0;
        effect.settings.release_ms = 0.0;

        let samples = vec![1.0_f32, -1.0, 1.0, -1.0];
        let output = effect.process(&samples, &context(2), false);
        assert_eq!(output.len(), samples.len());
        assert!(output.iter().all(|value| value.is_finite()));
        assert!(output.iter().any(|value| value.abs() < 1.0));
    }

    #[test]
    fn limiter_split_matches_single_pass() {
        let mut settings = LimiterEffect::default();
        settings.enabled = true;
        settings.settings.threshold_db = -6.0;
        settings.settings.knee_width_db = 1.0;
        settings.settings.attack_ms = 0.0;
        settings.settings.release_ms = 0.0;

        let samples = vec![1.0_f32, -1.0, 0.8, -0.8, 0.6, -0.6, 0.4, -0.4];

        let mut effect_full = settings.clone();
        let out_full = effect_full.process(&samples, &context(2), false);

        let mut effect_split = settings;
        let mid = samples.len() / 2;
        let mut out_split = effect_split.process(&samples[..mid], &context(2), false);
        out_split.extend(effect_split.process(&samples[mid..], &context(2), false));

        assert_eq!(out_full.len(), out_split.len());
        for (a, b) in out_full.iter().zip(out_split.iter()) {
            assert!(approx_eq(*a, *b, 1e-5));
        }
    }
}
