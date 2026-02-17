//! High-level playback controller for the Proteus library.

mod controls;
mod effects;
mod runtime;
mod settings;

use rodio::Sink;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use crate::container::prot::{PathsTrack, Prot};
use crate::diagnostics::reporter::Reporter;
use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::playback::output_meter::OutputMeter;
use crate::{
    container::info::Info,
    dsp::effects::AudioEffect,
    playback::engine::{DspChainMetrics, InlineEffectsUpdate, PlaybackBufferSettings},
};

/// High-level playback state for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Init,
    Resuming,
    Playing,
    Pausing,
    Paused,
    Stopping,
    Stopped,
    Finished,
}

/// Snapshot of convolution reverb settings for UI consumers.
#[derive(Debug, Clone, Copy)]
pub struct ReverbSettingsSnapshot {
    pub enabled: bool,
    pub dry_wet: f32,
}

const OUTPUT_METER_REFRESH_HZ: f32 = 30.0;
const OUTPUT_STREAM_OPEN_RETRIES: usize = 20;
const OUTPUT_STREAM_OPEN_RETRY_MS: u64 = 100;

/// Primary playback controller.
///
/// `Player` owns the playback threads, buffering state, and runtime settings
/// such as volume and reverb configuration.
#[derive(Clone)]
pub struct Player {
    pub info: Info,
    pub finished_tracks: Arc<Mutex<Vec<i32>>>,
    pub ts: Arc<Mutex<f64>>,
    state: Arc<Mutex<PlayerState>>,
    abort: Arc<AtomicBool>,
    playback_thread_exists: Arc<AtomicBool>,
    playback_id: Arc<AtomicU64>,
    duration: Arc<Mutex<f64>>,
    prot: Arc<Mutex<Prot>>,
    audio_heard: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    sink: Arc<Mutex<Sink>>,
    reporter: Option<Arc<Mutex<Reporter>>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    effects: Arc<Mutex<Vec<AudioEffect>>>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    effects_reset: Arc<AtomicU64>,
    output_meter: Arc<Mutex<OutputMeter>>,
    buffering_done: Arc<AtomicBool>,
    last_chunk_ms: Arc<AtomicU64>,
    last_time_update_ms: Arc<AtomicU64>,
    next_resume_fade_ms: Arc<Mutex<Option<f32>>>,
    impulse_response_override: Option<ImpulseResponseSpec>,
    impulse_response_tail_override: Option<f32>,
}

impl Player {
    /// Create a new player for a single container path.
    pub fn new(file_path: &String) -> Self {
        Self::new_from_path_or_paths(Some(file_path), None)
    }

    /// Create a new player for a set of standalone file paths.
    pub fn new_from_file_paths(file_paths: Vec<PathsTrack>) -> Self {
        Self::new_from_path_or_paths(None, Some(file_paths))
    }

    /// Create a new player for a set of standalone file paths.
    pub fn new_from_file_paths_legacy(file_paths: Vec<Vec<String>>) -> Self {
        Self::new_from_path_or_paths(
            None,
            Some(
                file_paths
                    .into_iter()
                    .map(PathsTrack::new_from_file_paths)
                    .collect(),
            ),
        )
    }

    /// Create a player from either a container path or standalone file paths.
    pub fn new_from_path_or_paths(path: Option<&String>, paths: Option<Vec<PathsTrack>>) -> Self {
        let (prot, info) = match path {
            Some(path) => {
                let prot = Arc::new(Mutex::new(Prot::new(path)));
                let info = Info::new(path.clone());
                (prot, info)
            }
            None => {
                let prot = Arc::new(Mutex::new(Prot::new_from_file_paths(paths.unwrap())));
                let locked_prot = prot.lock().unwrap();
                let info = Info::new_from_file_paths(locked_prot.get_file_paths_dictionary());
                drop(locked_prot);
                (prot, info)
            }
        };

        let (sink, _queue) = Sink::new();
        let sink: Arc<Mutex<Sink>> = Arc::new(Mutex::new(sink));

        let channels = info.channels as usize;
        let sample_rate = info.sample_rate;
        let effects = {
            let prot_locked = prot.lock().unwrap();
            match prot_locked.get_effects() {
                Some(effects) => Arc::new(Mutex::new(effects)),
                None => Arc::new(Mutex::new(vec![])),
            }
        };

        let mut this = Self {
            info,
            finished_tracks: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(PlayerState::Stopped)),
            abort: Arc::new(AtomicBool::new(false)),
            ts: Arc::new(Mutex::new(0.0)),
            playback_thread_exists: Arc::new(AtomicBool::new(true)),
            playback_id: Arc::new(AtomicU64::new(0)),
            duration: Arc::new(Mutex::new(0.0)),
            audio_heard: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(Mutex::new(0.8)),
            sink,
            prot,
            reporter: None,
            buffer_settings: Arc::new(Mutex::new(PlaybackBufferSettings::new(20.0))),
            effects,
            inline_effects_update: Arc::new(Mutex::new(None)),
            dsp_metrics: Arc::new(Mutex::new(DspChainMetrics::default())),
            effects_reset: Arc::new(AtomicU64::new(0)),
            output_meter: Arc::new(Mutex::new(OutputMeter::new(
                channels,
                sample_rate,
                OUTPUT_METER_REFRESH_HZ,
            ))),
            buffering_done: Arc::new(AtomicBool::new(false)),
            last_chunk_ms: Arc::new(AtomicU64::new(0)),
            last_time_update_ms: Arc::new(AtomicU64::new(0)),
            next_resume_fade_ms: Arc::new(Mutex::new(None)),
            impulse_response_override: None,
            impulse_response_tail_override: None,
        };

        this.initialize_thread(None);

        this
    }
}
