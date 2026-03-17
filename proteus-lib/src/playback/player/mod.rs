//! High-level playback controller for the Proteus library.
//!
//! `Player` is the primary integration point for consumers that need to load a
//! container or file list, control transport state, and inspect DSP/runtime
//! telemetry. Implementation details are split into focused submodules:
//! - `controls`: transport operations and lifecycle orchestration.
//! - `effects`: DSP-chain and metering controls.
//! - `settings`: runtime tuning and debug surface.
//! - `runtime`: internal playback thread bootstrap and worker loop.

mod builder;
mod controls;
mod effects;
mod lifecycle;
mod runtime;
mod settings;
mod state;

use rodio::{OutputStream, Sink};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::container::prot::{PathsTrack, Prot, ProtError};
use crate::diagnostics::reporter::Reporter;
use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;
use crate::playback::output_meter::OutputMeter;
use crate::{
    container::info::Info,
    dsp::effects::AudioEffect,
    playback::engine::{
        DspChainMetrics, EffectSettingsCommand, InlineEffectsUpdate, InlineTrackMixUpdate,
        PlaybackBufferSettings,
    },
};

/// High-level playback state for the player.
///
/// The public transport methods mostly request transitions (`Pausing`,
/// `Resuming`, `Stopping`) that are resolved on the playback thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    /// Player has been created but playback has not yet started.
    Init,
    /// A resume has been requested; the playback thread is fading audio back in.
    Resuming,
    /// Audio is actively playing.
    Playing,
    /// A pause has been requested; the playback thread is fading audio out.
    Pausing,
    /// Playback is paused and the audio sink is silent.
    Paused,
    /// A stop has been requested; the playback thread is winding down.
    Stopping,
    /// Playback has stopped and the playback position is reset to the start.
    Stopped,
    /// Playback reached end-of-stream and completed normally.
    Finished,
}

/// Action to apply automatically when playback reaches end-of-stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOfStreamAction {
    /// Stop playback and reset the playback time to `0.0`.
    Stop,
    /// Pause playback and keep the playback time at the end position.
    Pause,
}

/// Initialization options for [`Player`].
#[derive(Debug, Clone, Copy)]
pub struct PlayerInitOptions {
    /// End-of-stream transport action.
    pub end_of_stream_action: EndOfStreamAction,
}

impl Default for PlayerInitOptions {
    fn default() -> Self {
        Self {
            end_of_stream_action: EndOfStreamAction::Stop,
        }
    }
}

/// Error produced when building a [`Player`] from invalid source inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerInitError {
    /// Neither container path nor standalone track paths were provided.
    MissingSource,
    /// Both container path and standalone track paths were provided.
    AmbiguousSource,
    /// Failed to initialize the underlying `.prot` container.
    ProtInitialization(ProtError),
}

impl std::fmt::Display for PlayerInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSource => write!(f, "player source input is required"),
            Self::AmbiguousSource => {
                write!(
                    f,
                    "player source input must be exactly one of path or file paths"
                )
            }
            Self::ProtInitialization(err) => write!(f, "player source init failed: {}", err),
        }
    }
}

impl std::error::Error for PlayerInitError {}

/// Source input used to initialize a [`Player`].
#[derive(Debug, Clone)]
pub enum PlayerSource {
    /// Playback from a `.prot`/`.mka` container path.
    ContainerPath(String),
    /// Playback from standalone grouped track paths.
    FilePaths(Vec<PathsTrack>),
}

/// Snapshot of active reverb settings for UI consumers.
///
/// Values are derived from the first matching reverb in the current effect
/// chain, with precedence handled in `effects::get_reverb_settings`.
#[derive(Debug, Clone, Copy)]
pub struct ReverbSettingsSnapshot {
    /// Whether the reverb effect is currently enabled.
    pub enabled: bool,
    /// Dry/wet mix ratio (0.0 = fully dry, 1.0 = fully wet).
    pub dry_wet: f32,
}

const OUTPUT_METER_REFRESH_HZ: f32 = 30.0;
const OUTPUT_STREAM_OPEN_RETRIES: usize = 20;
const OUTPUT_STREAM_OPEN_RETRY_MS: u64 = 100;

/// Primary playback controller.
///
/// `Player` owns the playback threads, buffering state, and runtime settings
/// such as volume and reverb configuration.
pub struct Player {
    /// Metadata describing the loaded container or file list.
    info: Info,
    /// Track IDs that have decoded all samples and reached end-of-stream.
    finished_tracks: Arc<Mutex<Vec<i32>>>,
    /// Current playback position in seconds, updated by the playback thread.
    ts: Arc<Mutex<f64>>,
    state: Arc<Mutex<PlayerState>>,
    abort: Arc<AtomicBool>,
    playback_thread_exists: Arc<AtomicBool>,
    playback_thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    playback_id: Arc<AtomicU64>,
    duration: Arc<Mutex<f64>>,
    prot: Arc<Mutex<Prot>>,
    audio_heard: Arc<AtomicBool>,
    play_command_ms: Arc<AtomicU64>,
    volume: Arc<Mutex<f32>>,
    sink: Arc<Mutex<Sink>>,
    #[allow(clippy::arc_with_non_send_sync)]
    output_stream: Arc<Mutex<Option<OutputStream>>>,
    reporter: Option<Arc<Mutex<Reporter>>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    effects: Arc<Mutex<Vec<AudioEffect>>>,
    effect_settings_commands: Arc<Mutex<Vec<EffectSettingsCommand>>>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    effects_reset: Arc<AtomicU64>,
    output_meter: Arc<Mutex<OutputMeter>>,
    buffering_done: Arc<AtomicBool>,
    last_chunk_ms: Arc<AtomicU64>,
    last_time_update_ms: Arc<AtomicU64>,
    next_resume_fade_ms: Arc<Mutex<Option<f32>>>,
    end_of_stream_action: Arc<Mutex<EndOfStreamAction>>,
    handle_count: Arc<AtomicUsize>,
    shutdown_once: Arc<AtomicBool>,
    impulse_response_override: Option<ImpulseResponseSpec>,
    impulse_response_tail_override: Option<f32>,
}

impl Clone for Player {
    fn clone(&self) -> Self {
        self.handle_count.fetch_add(1, Ordering::Relaxed);
        Self {
            info: self.info.clone(),
            finished_tracks: self.finished_tracks.clone(),
            ts: self.ts.clone(),
            state: self.state.clone(),
            abort: self.abort.clone(),
            playback_thread_exists: self.playback_thread_exists.clone(),
            playback_thread_handle: self.playback_thread_handle.clone(),
            playback_id: self.playback_id.clone(),
            duration: self.duration.clone(),
            prot: self.prot.clone(),
            audio_heard: self.audio_heard.clone(),
            play_command_ms: self.play_command_ms.clone(),
            volume: self.volume.clone(),
            sink: self.sink.clone(),
            output_stream: self.output_stream.clone(),
            reporter: self.reporter.clone(),
            buffer_settings: self.buffer_settings.clone(),
            effects: self.effects.clone(),
            effect_settings_commands: self.effect_settings_commands.clone(),
            inline_effects_update: self.inline_effects_update.clone(),
            inline_track_mix_updates: self.inline_track_mix_updates.clone(),
            dsp_metrics: self.dsp_metrics.clone(),
            effects_reset: self.effects_reset.clone(),
            output_meter: self.output_meter.clone(),
            buffering_done: self.buffering_done.clone(),
            last_chunk_ms: self.last_chunk_ms.clone(),
            last_time_update_ms: self.last_time_update_ms.clone(),
            next_resume_fade_ms: self.next_resume_fade_ms.clone(),
            end_of_stream_action: self.end_of_stream_action.clone(),
            handle_count: self.handle_count.clone(),
            shutdown_once: self.shutdown_once.clone(),
            impulse_response_override: self.impulse_response_override.clone(),
            impulse_response_tail_override: self.impulse_response_tail_override,
        }
    }
}

#[allow(clippy::arc_with_non_send_sync)]
pub(in crate::playback::player) fn default_output_stream_handle() -> Arc<Mutex<Option<OutputStream>>>
{
    Arc::new(Mutex::new(None))
}

impl Player {
    /// Create a new player from a typed input source.
    ///
    /// # Arguments
    ///
    /// * `source` - Typed source for player initialization.
    ///
    /// # Panics
    ///
    /// Panics if player initialization fails. Prefer
    /// [`Self::try_from_source_with_options`] for fallible construction.
    pub fn from_source(source: PlayerSource) -> Self {
        Self::from_source_with_options(source, PlayerInitOptions::default())
    }

    /// Create a new player from a typed input source and options.
    ///
    /// # Arguments
    ///
    /// * `source` - Typed source for player initialization.
    /// * `options` - Player initialization options.
    ///
    /// # Panics
    ///
    /// Panics if player initialization fails. Prefer
    /// [`Self::try_from_source_with_options`] for fallible construction.
    pub fn from_source_with_options(source: PlayerSource, options: PlayerInitOptions) -> Self {
        Self::try_from_source_with_options(source, options)
            .unwrap_or_else(|err| panic!("Player initialization failed: {}", err))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        lifecycle::drop_cleanup(self);
    }
}
