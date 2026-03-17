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
