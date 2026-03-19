//! Data types shared by the mix thread implementation.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;

use super::super::state::{DspChainMetrics, PlaybackBufferSettings};
use super::super::{InlineEffectsUpdate, InlineTrackMixUpdate};

/// Incremental effect settings change pushed from the control path.
///
/// Commands are drained by the mix thread at chunk boundaries and applied to
/// the real-time-owned effect chain without requiring a full chain replacement.
#[derive(Debug, Clone)]
pub enum EffectSettingsCommand {
    /// Toggle the enabled state of supported reverb effects.
    SetReverbEnabled(bool),
    /// Update the reverb wet/dry mix ratio.
    SetReverbMix(f32),
    /// Update a single parameter on the effect at a given chain index.
    SetEffectParameter {
        /// Index into the effect chain.
        effect_index: usize,
        /// Which parameter to update.
        parameter: EffectParameter,
    },
    /// Toggle enabled state for any effect by chain index.
    SetEffectEnabled {
        /// Index into the effect chain.
        effect_index: usize,
        /// New enabled state.
        enabled: bool,
    },
}

/// Identifies a specific parameter on an effect for targeted inline updates.
#[derive(Debug, Clone)]
pub enum EffectParameter {
    /// Gain effect linear multiplier.
    Gain(f32),
    /// Pan position in `[-1.0, 1.0]`.
    Pan(f32),
    /// Reverb/convolution dry/wet mix in `[0.0, 1.0]`.
    ReverbMix(f32),
    /// Distortion pre-gain multiplier.
    DistortionGain(f32),
    /// Distortion clipping threshold.
    DistortionThreshold(f32),
    /// Low-pass filter cutoff frequency in Hz.
    LowPassFreqHz(u32),
    /// Low-pass filter Q factor.
    LowPassQ(f32),
    /// High-pass filter cutoff frequency in Hz.
    HighPassFreqHz(u32),
    /// High-pass filter Q factor.
    HighPassQ(f32),
    /// Compressor threshold in dBFS.
    CompressorThresholdDb(f32),
    /// Compressor ratio.
    CompressorRatio(f32),
    /// Compressor attack time in milliseconds.
    CompressorAttackMs(f32),
    /// Compressor release time in milliseconds.
    CompressorReleaseMs(f32),
    /// Compressor makeup gain in dB.
    CompressorMakeupDb(f32),
    /// Limiter threshold in dBFS.
    LimiterThresholdDb(f32),
    /// Limiter knee width in dB.
    LimiterKneeWidthDb(f32),
    /// Limiter attack time in milliseconds.
    LimiterAttackMs(f32),
    /// Limiter release time in milliseconds.
    LimiterReleaseMs(f32),
}

/// Arguments required to spawn the mixing thread.
pub struct MixThreadArgs {
    pub audio_info: crate::container::info::Info,
    pub buffer_notify: Arc<std::sync::Condvar>,
    pub effects_reset: Arc<AtomicU64>,
    pub inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub prot: Arc<Mutex<Prot>>,
    pub abort: Arc<AtomicBool>,
    pub start_time: f64,
    pub buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    pub effect_settings_commands: Arc<Mutex<Vec<EffectSettingsCommand>>>,
}

/// Active in-progress inline effect transition state.
#[derive(Debug, Clone)]
pub(super) struct ActiveInlineTransition {
    pub(super) old_effects: Vec<AudioEffect>,
    pub(super) new_effects: Vec<AudioEffect>,
    pub(super) total_samples: usize,
    pub(super) remaining_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::ActiveInlineTransition;

    #[test]
    fn active_inline_transition_tracks_sample_budget() {
        let transition = ActiveInlineTransition {
            old_effects: Vec::new(),
            new_effects: Vec::new(),
            total_samples: 256,
            remaining_samples: 128,
        };
        assert_eq!(transition.total_samples, 256);
        assert_eq!(transition.remaining_samples, 128);
        assert!(transition.old_effects.is_empty());
        assert!(transition.new_effects.is_empty());
    }
}
