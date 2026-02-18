//! High-level playback controller for the Proteus library.
//!
//! `Player` is the primary integration point for consumers that need to load a
//! container or file list, control transport state, and inspect DSP/runtime
//! telemetry. Implementation details are split into focused submodules:
//! - `controls`: transport operations and lifecycle orchestration.
//! - `effects`: DSP-chain and metering controls.
//! - `settings`: runtime tuning and debug surface.
//! - `runtime`: internal playback thread bootstrap and worker loop.

mod controls;
mod effects;
mod runtime;
mod settings;

use rodio::Sink;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::container::prot::{PathsTrack, Prot};
use crate::diagnostics::reporter::Reporter;
use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::playback::output_meter::OutputMeter;
use crate::{
    container::info::Info,
    dsp::effects::AudioEffect,
    playback::engine::{
        DspChainMetrics, InlineEffectsUpdate, InlineTrackMixUpdate, PlaybackBufferSettings,
    },
};

/// High-level playback state for the player.
///
/// The public transport methods mostly request transitions (`Pausing`,
/// `Resuming`, `Stopping`) that are resolved on the playback thread.
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

/// Snapshot of active reverb settings for UI consumers.
///
/// Values are derived from the first matching reverb in the current effect
/// chain, with precedence handled in `effects::get_reverb_settings`.
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
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
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
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to a `.prot`/`.mka` container file.
    pub fn new(file_path: &String) -> Self {
        Self::new_from_path_or_paths(Some(file_path), None)
    }

    /// Create a new player for a set of standalone file paths.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Pre-normalized track path groups.
    pub fn new_from_file_paths(file_paths: Vec<PathsTrack>) -> Self {
        Self::new_from_path_or_paths(None, Some(file_paths))
    }

    /// Create a new player for legacy standalone file-path groups.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Legacy track grouping shape where each inner vector is
    ///   interpreted as one track candidate set.
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
    ///
    /// Exactly one input source is expected. `path` takes precedence when
    /// provided; otherwise `paths` is used for file-based playback.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional container path.
    /// * `paths` - Optional standalone track path groups.
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
            inline_track_mix_updates: Arc::new(Mutex::new(Vec::new())),
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

impl Drop for Player {
    fn drop(&mut self) {
        // `Player` is `Clone` and shares state through Arcs. Only the final
        // handle should tear down the shared runtime resources.
        if Arc::strong_count(&self.state) != 1 {
            return;
        }

        if let Some(reporter) = self.reporter.take() {
            reporter.lock().unwrap().stop();
        }

        if self
            .playback_thread_exists
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.kill_current();
        } else {
            self.abort.store(true, Ordering::SeqCst);
        }

        {
            let sink = self.sink.lock().unwrap();
            sink.stop();
            sink.clear();
        }

        {
            let mut finished_tracks = self.finished_tracks.lock().unwrap();
            finished_tracks.clear();
            finished_tracks.shrink_to_fit();
        }

        {
            let mut effects = self.effects.lock().unwrap();
            effects.clear();
            effects.shrink_to_fit();
        }

        {
            let mut inline_effects_update = self.inline_effects_update.lock().unwrap();
            *inline_effects_update = None;
        }

        {
            let mut inline_track_mix_updates = self.inline_track_mix_updates.lock().unwrap();
            inline_track_mix_updates.clear();
            inline_track_mix_updates.shrink_to_fit();
        }

        {
            let mut dsp_metrics = self.dsp_metrics.lock().unwrap();
            *dsp_metrics = DspChainMetrics::default();
        }

        {
            let mut output_meter = self.output_meter.lock().unwrap();
            output_meter.reset();
        }

        *self.duration.lock().unwrap() = 0.0;
        *self.ts.lock().unwrap() = 0.0;
        *self.next_resume_fade_ms.lock().unwrap() = None;
        self.buffering_done.store(false, Ordering::Relaxed);
        self.last_chunk_ms.store(0, Ordering::Relaxed);
        self.last_time_update_ms.store(0, Ordering::Relaxed);
        self.audio_heard.store(false, Ordering::Relaxed);
        self.impulse_response_override = None;
        self.impulse_response_tail_override = None;
    }
}
