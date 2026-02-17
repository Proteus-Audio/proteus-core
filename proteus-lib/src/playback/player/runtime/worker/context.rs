//! Shared runtime context captured at thread spawn time.

use rodio::Sink;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;
use crate::playback::engine::{DspChainMetrics, InlineEffectsUpdate, PlaybackBufferSettings};
use crate::playback::output_meter::OutputMeter;

use super::super::super::PlayerState;

/// Captured shared state passed from `Player::initialize_thread` into the
/// detached worker thread.
pub(in crate::playback::player::runtime) struct ThreadContext {
    pub(in crate::playback::player::runtime) play_state: Arc<Mutex<PlayerState>>,
    pub(in crate::playback::player::runtime) abort: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) playback_thread_exists: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) playback_id_atomic: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) time_passed: Arc<Mutex<f64>>,
    pub(in crate::playback::player::runtime) duration: Arc<Mutex<f64>>,
    pub(in crate::playback::player::runtime) prot: Arc<Mutex<Prot>>,
    pub(in crate::playback::player::runtime) buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub(in crate::playback::player::runtime) buffer_settings_for_state:
        Arc<Mutex<PlaybackBufferSettings>>,
    pub(in crate::playback::player::runtime) effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub(in crate::playback::player::runtime) inline_effects_update:
        Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub(in crate::playback::player::runtime) dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    pub(in crate::playback::player::runtime) dsp_metrics_for_sink: Arc<Mutex<DspChainMetrics>>,
    pub(in crate::playback::player::runtime) effects_reset: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) output_meter: Arc<Mutex<OutputMeter>>,
    pub(in crate::playback::player::runtime) audio_info: Info,
    pub(in crate::playback::player::runtime) next_resume_fade_ms: Arc<Mutex<Option<f32>>>,
    pub(in crate::playback::player::runtime) audio_heard: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) volume: Arc<Mutex<f32>>,
    pub(in crate::playback::player::runtime) sink_mutex: Arc<Mutex<Sink>>,
    pub(in crate::playback::player::runtime) buffer_done_thread_flag: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) last_chunk_ms: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) last_time_update_ms: Arc<AtomicU64>,
}
