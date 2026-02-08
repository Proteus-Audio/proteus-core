//! Convolution reverb effect wrapper for the DSP chain.

use std::path::{Path, PathBuf};

use log::{info, warn};
use serde::{Deserialize, Serialize};

use super::EffectContext;

pub mod convolution;
pub mod impulse_response;
pub mod reverb;
mod spec;

pub use spec::{parse_impulse_response_string, ImpulseResponseSpec};
pub(crate) use spec::{parse_impulse_response_spec, parse_impulse_response_tail_db};

const DEFAULT_DRY_WET: f32 = 0.000001;
const DEFAULT_TAIL_DB: f32 = -60.0;
const REVERB_BATCH_BLOCKS: usize = 2;

/// Serialized configuration for convolution reverb impulse response selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvolutionReverbSettings {
    pub impulse_response: Option<String>,
    pub impulse_response_attachment: Option<String>,
    pub impulse_response_path: Option<String>,
    pub impulse_response_tail_db: Option<f32>,
    pub impulse_response_tail: Option<f32>,
}

impl Default for ConvolutionReverbSettings {
    fn default() -> Self {
        Self {
            impulse_response: None,
            impulse_response_attachment: None,
            impulse_response_path: None,
            impulse_response_tail_db: None,
            impulse_response_tail: None,
        }
    }
}

/// Configured convolution reverb effect with runtime state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvolutionReverbEffect {
    pub enabled: bool,
    pub dry_wet: f32,
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

    /// Process interleaved samples through the reverb.
    ///
    /// # Arguments
    /// - `samples`: Interleaved input samples.
    /// - `context`: Environment details (sample rate, channels, etc.).
    /// - `drain`: When true, flush buffered tail data if present.
    ///
    /// # Returns
    /// Processed interleaved samples.
    pub fn process(&mut self, samples: &[f32], context: &EffectContext, drain: bool) -> Vec<f32> {
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

    /// Clear all internal buffers and convolution history.
    ///
    /// # Returns
    /// Nothing.
    pub fn reset_state(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.reset();
        }
        self.state = None;
        self.resolved_config = None;
    }

    fn ensure_state(&mut self, context: &EffectContext) {
        let config = self.resolve_config(context);
        if self.resolved_config.as_ref() == Some(&config) && self.state.is_some() {
            return;
        }

        let reverb = build_reverb_with_impulse_response(
            config.channels,
            self.dry_wet,
            config.impulse_spec.clone(),
            config.container_path.as_deref(),
            config.tail_db,
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
}

impl ConvolutionReverbState {
    fn new(mut reverb: reverb::Reverb) -> Self {
        info!("Using Convolution Reverb!");
        let block_samples = reverb.block_size_samples();
        reverb.set_dry_wet(DEFAULT_DRY_WET);
        Self {
            reverb,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
            block_out: Vec::new(),
            block_samples,
        }
    }

    fn reset(&mut self) {
        self.reverb.clear_state();
        self.input_buffer.clear();
        self.output_buffer.clear();
        self.block_out.clear();
        self.block_samples = self.reverb.block_size_samples();
    }

    fn process(&mut self, samples: &[f32], drain: bool) -> Vec<f32> {
        if samples.is_empty() {
            if drain && !self.output_buffer.is_empty() {
                let out = self.output_buffer.clone();
                self.output_buffer.clear();
                return out;
            }
            return Vec::new();
        }

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

        let chunk_len = samples.len();
        if self.output_buffer.len() < chunk_len {
            let mut out = self.output_buffer.clone();
            if out.len() < chunk_len {
                out.resize(chunk_len, 0.0);
            }
            self.output_buffer.clear();
            return out;
        }

        self.output_buffer.drain(0..chunk_len).collect()
    }
}

fn build_reverb_with_impulse_response(
    channels: usize,
    dry_wet: f32,
    impulse_spec: Option<ImpulseResponseSpec>,
    container_path: Option<&str>,
    tail_db: f32,
) -> Option<reverb::Reverb> {
    let impulse_spec = impulse_spec?;

    use self::impulse_response::{
        load_impulse_response_from_file_with_tail,
        load_impulse_response_from_prot_attachment_with_tail,
    };

    let result = match impulse_spec {
        ImpulseResponseSpec::Attachment(name) => container_path
            .ok_or_else(|| "missing container path for attachment".to_string())
            .and_then(|path| {
                load_impulse_response_from_prot_attachment_with_tail(path, &name, Some(tail_db))
                    .map_err(|err| err.to_string())
            }),
        ImpulseResponseSpec::FilePath(path) => {
            let resolved_path = resolve_impulse_response_path(container_path, &path);
            if resolved_path.exists() {
                load_impulse_response_from_file_with_tail(&resolved_path, Some(tail_db))
                    .map_err(|err| err.to_string())
            } else {
                match container_path {
                    Some(container_path) => {
                        let fallback_name = Path::new(&path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.to_string());
                        if let Some(fallback_name) = fallback_name {
                            load_impulse_response_from_prot_attachment_with_tail(
                                container_path,
                                &fallback_name,
                                Some(tail_db),
                            )
                            .map_err(|err| err.to_string())
                        } else {
                            Err(format!(
                                "impulse response path not found: {}",
                                resolved_path.display()
                            ))
                        }
                    }
                    None => Err(format!(
                        "impulse response path not found: {}",
                        resolved_path.display()
                    )),
                }
            }
        }
    };

    match result {
        Ok(impulse_response) => Some(reverb::Reverb::new_with_impulse_response(
            channels,
            dry_wet,
            &impulse_response,
        )),
        Err(err) => {
            warn!(
                "Failed to load impulse response ({}); skipping convolution reverb.",
                err
            );
            None
        }
    }
}

fn resolve_impulse_response_path(container_path: Option<&str>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(container_path) = container_path {
        if let Some(parent) = Path::new(container_path).parent() {
            return parent.join(path);
        }
    }

    path.to_path_buf()
}

impl ConvolutionReverbSettings {
    /// Resolve a tail trim value, falling back to the default.
    pub fn tail_db_or_default(&self) -> f32 {
        self.impulse_response_tail_db
            .or(self.impulse_response_tail)
            .unwrap_or(DEFAULT_TAIL_DB)
    }
}
