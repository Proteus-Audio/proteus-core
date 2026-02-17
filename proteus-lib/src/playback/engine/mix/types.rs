//! Data types shared by the mix thread implementation.

use dasp_ring_buffer::Bounded;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use crate::audio::buffer::TrackBuffer;
use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;

use super::super::state::{DspChainMetrics, PlaybackBufferSettings};
use super::super::InlineEffectsUpdate;

/// Arguments required to spawn the mixing thread.
pub struct MixThreadArgs {
    pub audio_info: crate::container::info::Info,
    pub buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    pub buffer_notify: Arc<std::sync::Condvar>,
    pub effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    pub track_weights: Arc<Mutex<HashMap<u16, f32>>>,
    pub track_channel_gains: Arc<Mutex<HashMap<u16, Vec<f32>>>>,
    pub effects_reset: Arc<AtomicU64>,
    pub inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub prot: Arc<Mutex<Prot>>,
    pub abort: Arc<AtomicBool>,
    pub start_time: f64,
    pub buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub dsp_metrics: Arc<Mutex<DspChainMetrics>>,
}

/// Active in-progress inline effect transition state.
#[derive(Debug, Clone)]
pub(super) struct ActiveInlineTransition {
    pub(super) old_effects: Vec<AudioEffect>,
    pub(super) new_effects: Vec<AudioEffect>,
    pub(super) total_samples: usize,
    pub(super) remaining_samples: usize,
}
