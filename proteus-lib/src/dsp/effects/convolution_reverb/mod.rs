//! Convolution reverb effect wrapper for the DSP chain.
//!
//! Impulse response loading, caching, and reverb kernel construction live in
//! `ir_loader`. The effect struct, its `DspEffect` impl, and the runtime
//! buffering state are defined here.

use log::info;
use serde::{Deserialize, Serialize};

use super::EffectContext;

pub mod convolution;
pub mod impulse_response;
mod ir_loader;
pub mod reverb;
mod spec;

pub use ir_loader::clear_global_caches;
pub use spec::{parse_impulse_response_string, ImpulseResponseSpec};

pub(crate) const DEFAULT_DRY_WET: f32 = 0.000001;
const DEFAULT_TAIL_DB: f32 = -60.0;
pub(crate) const REVERB_BATCH_BLOCKS: usize = 2;
const DRAIN_MAX_BLOCKS: usize = 128;
const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;
const DRAIN_SILENT_BLOCKS_TO_STOP: usize = 2;

/// Preferred processing batch size in interleaved samples for the reverb.
pub fn preferred_batch_samples(channels: usize) -> usize {
    reverb::preferred_batch_samples(channels)
}

/// Serialized configuration for convolution reverb impulse response selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvolutionReverbSettings {
    /// Inline IR identifier or attachment name (primary field, checked first).
    pub impulse_response: Option<String>,
    /// Name of the IR embedded as a Matroska attachment (legacy alias).
    pub impulse_response_attachment: Option<String>,
    /// Filesystem path to an external IR audio file (legacy alias).
    pub impulse_response_path: Option<String>,
    /// dB level below peak at which the IR tail is considered silent and truncated.
    pub impulse_response_tail_db: Option<f32>,
    /// Legacy alias for `impulse_response_tail_db`.
    pub impulse_response_tail: Option<f32>,
}

impl ConvolutionReverbSettings {
    /// Resolve a tail trim value, falling back to the default.
    pub fn tail_db_or_default(&self) -> f32 {
        self.impulse_response_tail_db
            .or(self.impulse_response_tail)
            .unwrap_or(DEFAULT_TAIL_DB)
    }
}

/// Configured convolution reverb effect with runtime state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvolutionReverbEffect {
    /// Whether the effect is active; when `false` samples pass through unmodified.
    pub enabled: bool,
    /// Dry/wet mix ratio (0.0 = fully dry, 1.0 = fully wet).
    #[serde(alias = "wet_dry", alias = "mix")]
    pub dry_wet: f32,
    /// Impulse response selection and tail configuration.
    #[serde(flatten)]
    pub settings: ConvolutionReverbSettings,
    #[serde(skip)]
    state: Option<ConvolutionReverbState>,
    #[serde(skip)]
    resolved_config: Option<ResolvedConfig>,
}

impl std::fmt::Debug for ConvolutionReverbEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvolutionReverbEffect")
            .field("enabled", &self.enabled)
            .field("dry_wet", &self.dry_wet)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Default for ConvolutionReverbEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            dry_wet: DEFAULT_DRY_WET,
            settings: ConvolutionReverbSettings::default(),
            state: None,
            resolved_config: None,
        }
    }
}

impl crate::dsp::effects::core::DspEffect for ConvolutionReverbEffect {
    fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
        self.ensure_state(context);
        if !self.enabled || self.dry_wet <= 0.0 {
            return samples.to_vec();
        }

        let Some(state) = self.state.as_mut() else {
            return samples.to_vec();
        };

        state.reverb.set_dry_wet(self.dry_wet);
        state.process(samples, drain)
    }

    fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
        self.resolved_config = None;
    }

    fn warm_up(&mut self, context: &EffectContext) {
        let _ = self.process(&[], context, false);
    }
}

impl ConvolutionReverbEffect {
    /// Create a new convolution reverb effect.
    pub fn new(dry_wet: f32) -> Self {
        Self {
            dry_wet: dry_wet.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Return the stored impulse response settings.
    pub fn settings(&self) -> &ConvolutionReverbSettings {
        &self.settings
    }

    /// Mutable access to the stored impulse response settings.
    pub fn settings_mut(&mut self) -> &mut ConvolutionReverbSettings {
        &mut self.settings
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let config = self.resolve_config(context);
        if self.resolved_config.as_ref() == Some(&config) && self.state.is_some() {
            return;
        }

        let start = std::time::Instant::now();
        let reverb = ir_loader::build_reverb_with_impulse_response(
            config.channels,
            self.dry_wet,
            config.impulse_spec.clone(),
            config.container_path.as_deref(),
            config.tail_db,
        );
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        log::info!(
            "convolution reverb init: {:.2}ms (ir={:?} channels={})",
            elapsed_ms,
            config.impulse_spec,
            config.channels
        );

        self.state = reverb.map(ConvolutionReverbState::new);
        self.resolved_config = Some(config);
    }

    fn resolve_config(&self, context: &EffectContext) -> ResolvedConfig {
        let impulse_spec = self
            .settings
            .impulse_response
            .as_deref()
            .and_then(parse_impulse_response_string)
            .or_else(|| {
                self.settings
                    .impulse_response_attachment
                    .as_deref()
                    .and_then(parse_impulse_response_string)
            })
            .or_else(|| {
                self.settings
                    .impulse_response_path
                    .as_deref()
                    .and_then(parse_impulse_response_string)
            })
            .or_else(|| context.impulse_response_spec.clone());

        let tail_db = self
            .settings
            .impulse_response_tail_db
            .or(self.settings.impulse_response_tail)
            .unwrap_or(context.impulse_response_tail_db);

        ResolvedConfig {
            channels: context.channels,
            container_path: context.container_path.clone(),
            impulse_spec,
            tail_db,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ResolvedConfig {
    channels: usize,
    container_path: Option<String>,
    impulse_spec: Option<ImpulseResponseSpec>,
    tail_db: f32,
}

#[derive(Clone)]
struct ConvolutionReverbState {
    reverb: reverb::Reverb,
    input_buffer: Vec<f32>,
    output_buffer: Vec<f32>,
    block_out: Vec<f32>,
    block_samples: usize,
    tail_drained: bool,
}

impl ConvolutionReverbState {
    fn new(mut reverb: reverb::Reverb) -> Self {
        info!("using convolution reverb");
        let block_samples = reverb.block_size_samples();
        reverb.set_dry_wet(DEFAULT_DRY_WET);
        Self {
            reverb,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
            block_out: Vec::new(),
            block_samples,
            tail_drained: false,
        }
    }

    fn reset(&mut self) {
        self.reverb.clear_state();
        self.input_buffer.clear();
        self.output_buffer.clear();
        self.block_out.clear();
        self.block_samples = self.reverb.block_size_samples();
        self.tail_drained = false;
    }

    fn process(&mut self, samples: &[f32], drain: bool) -> Vec<f32> {
        if samples.is_empty() {
            if !drain {
                return Vec::new();
            }
            if self.tail_drained {
                return Vec::new();
            }

            let mut out = Vec::new();
            if !self.output_buffer.is_empty() {
                out.append(&mut self.output_buffer);
            }
            out.extend(self.drain_tail_blocks());
            self.tail_drained = true;
            return out;
        }

        self.tail_drained = false;

        if self.block_samples == 0 {
            return self.reverb.process(samples);
        }

        self.input_buffer.extend_from_slice(samples);
        let batch_samples = self.block_samples * REVERB_BATCH_BLOCKS;
        let should_flush = drain && !self.input_buffer.is_empty();
        while self.input_buffer.len() >= batch_samples || should_flush {
            let take = if self.input_buffer.len() >= batch_samples {
                batch_samples
            } else {
                self.input_buffer.len()
            };
            let block: Vec<f32> = self.input_buffer.drain(0..take).collect();
            self.reverb.process_into(&block, &mut self.block_out);
            self.output_buffer.extend_from_slice(&self.block_out);
            if take < batch_samples {
                break;
            }
        }

        // Keep output continuous for small chunks (e.g. around shuffle boundaries).
        // If batch processing did not yield enough samples yet, process the pending
        // input immediately instead of emitting silence.
        while self.output_buffer.len() < samples.len() && !self.input_buffer.is_empty() {
            let take = self.input_buffer.len().min(batch_samples.max(1));
            let block: Vec<f32> = self.input_buffer.drain(0..take).collect();
            self.reverb.process_into(&block, &mut self.block_out);
            self.output_buffer.extend_from_slice(&self.block_out);
        }

        let chunk_len = samples.len();
        if self.output_buffer.len() < chunk_len {
            let mut out: Vec<f32> = self.output_buffer.drain(..).collect();
            let out_len = out.len();
            if out_len < chunk_len {
                out.extend_from_slice(&samples[out_len..chunk_len]);
            }
            self.output_buffer.clear();
            return out;
        }

        self.output_buffer.drain(0..chunk_len).collect()
    }

    fn drain_tail_blocks(&mut self) -> Vec<f32> {
        if self.block_samples == 0 {
            return Vec::new();
        }

        let mut drained = Vec::new();
        let mut trailing_silent_blocks = 0usize;
        let silence = vec![0.0_f32; self.block_samples.max(1)];

        for _ in 0..DRAIN_MAX_BLOCKS {
            self.reverb.process_into(&silence, &mut self.block_out);
            if self.block_out.is_empty() {
                break;
            }

            let max_abs = self
                .block_out
                .iter()
                .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));

            if max_abs > DRAIN_SILENCE_EPSILON {
                trailing_silent_blocks = 0;
            } else {
                trailing_silent_blocks = trailing_silent_blocks.saturating_add(1);
            }

            drained.extend_from_slice(&self.block_out);

            if trailing_silent_blocks >= DRAIN_SILENT_BLOCKS_TO_STOP {
                break;
            }
        }

        drained
    }
}

#[cfg(test)]
mod tests {
    use super::{ConvolutionReverbEffect, ConvolutionReverbSettings, EffectContext};
    use crate::dsp::effects::core::DspEffect;

    #[test]
    fn tail_db_or_default_prefers_explicit_values() {
        let settings = ConvolutionReverbSettings {
            impulse_response_tail_db: Some(-24.0),
            impulse_response_tail: Some(-30.0),
            ..Default::default()
        };
        assert_eq!(settings.tail_db_or_default(), -24.0);
    }

    #[test]
    fn convolution_effect_passthrough_when_disabled() {
        let mut effect = ConvolutionReverbEffect::default();
        effect.enabled = false;
        let input = vec![0.2_f32, -0.2, 0.1, -0.1];
        let context = EffectContext {
            sample_rate: 48_000,
            channels: 2,
            container_path: None,
            impulse_response_spec: None,
            impulse_response_tail_db: -60.0,
        };
        let output = effect.process(&input, &context, false);
        assert_eq!(output, input);
    }
}
