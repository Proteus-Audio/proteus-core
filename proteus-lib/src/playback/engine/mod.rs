//! Playback mixing engine and buffer coordination.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc::Receiver, Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use rodio::buffer::SamplesBuffer;

use log::warn;

use crate::audio::buffer::{init_buffer_map, TrackBuffer};
use crate::container::prot::Prot;
use crate::playback::mutex_policy::{lock_invariant, lock_recoverable};

mod mix;
pub(crate) mod premix;
mod state;

pub use state::{DspChainMetrics, PlaybackBufferSettings};

pub use mix::EffectSettingsCommand;

use mix::{spawn_mix_thread, MixThreadArgs};

/// Request to update the active effects chain inline during playback.
#[derive(Debug, Clone)]
pub struct InlineEffectsUpdate {
    /// The new DSP effect chain to apply, in processing order.
    pub effects: Vec<crate::dsp::effects::AudioEffect>,
    /// Duration in milliseconds to crossfade between the old and new chain.
    pub transition_ms: f32,
}

impl InlineEffectsUpdate {
    /// Create an inline effect update request.
    pub fn new(effects: Vec<crate::dsp::effects::AudioEffect>, transition_ms: f32) -> Self {
        Self {
            effects,
            transition_ms: transition_ms.max(0.0),
        }
    }
}

/// Request to update per-slot track mix settings inline during playback.
#[derive(Debug, Clone, Copy)]
pub struct InlineTrackMixUpdate {
    /// Zero-based index of the track slot whose mix parameters are being updated.
    pub slot_index: usize,
    /// New linear gain level for the track (1.0 = unity).
    pub level: f32,
    /// New stereo pan position (−1.0 = full left, +1.0 = full right).
    pub pan: f32,
}

/// Shared initialization inputs for [`PlayerEngine`].
pub struct PlayerEngineConfig {
    /// Optional externally-owned abort flag; a new flag is created if `None`.
    pub abort_option: Option<Arc<AtomicBool>>,
    /// Wall-clock start time (in seconds) used to synchronize playback position.
    pub start_time: f64,
    /// Shared buffer configuration applied at engine startup and during playback.
    pub buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    /// Shared DSP effect chain applied to the final mix output.
    pub effects: Arc<Mutex<Vec<crate::dsp::effects::AudioEffect>>>,
    /// Shared structure into which the engine writes live DSP performance metrics.
    pub dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    /// Monotonic counter incremented each time the effect chain should be reset.
    pub effects_reset: Arc<AtomicU64>,
    /// Pending inline effects-chain swap to apply on the next mix cycle.
    pub inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    /// Pending per-track mix updates to apply on the next mix cycle.
    pub inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    /// Command queue for incremental effect settings changes from the control path.
    pub effect_settings_commands: Arc<Mutex<Vec<EffectSettingsCommand>>>,
}

/// Internal playback engine used by the high-level
/// [`crate::playback::player::Player`].
#[derive(Debug)]
pub struct PlayerEngine {
    /// Set of track IDs that have decoded all samples and reached end-of-stream.
    finished_tracks: Arc<Mutex<Vec<u16>>>,
    start_time: f64,
    abort: Arc<AtomicBool>,
    buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    buffer_notify: Arc<Condvar>,
    track_weights: Arc<Mutex<HashMap<u16, f32>>>,
    track_channel_gains: Arc<Mutex<HashMap<u16, Vec<f32>>>>,
    effects_reset: Arc<AtomicU64>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    prot: Arc<Mutex<Prot>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    effects: Arc<Mutex<Vec<crate::dsp::effects::AudioEffect>>>,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    effect_settings_commands: Arc<Mutex<Vec<EffectSettingsCommand>>>,
    mix_thread_handle: Option<JoinHandle<()>>,
}

impl PlayerEngine {
    /// Create a new engine for the given container and settings.
    pub fn new(prot: Arc<Mutex<Prot>>, config: PlayerEngineConfig) -> Self {
        let PlayerEngineConfig {
            abort_option,
            start_time,
            buffer_settings,
            effects,
            dsp_metrics,
            effects_reset,
            inline_effects_update,
            inline_track_mix_updates,
            effect_settings_commands,
        } = config;
        let buffer_map = init_buffer_map();
        let buffer_notify = Arc::new(Condvar::new());
        let track_weights = Arc::new(Mutex::new(HashMap::new()));
        let track_channel_gains = Arc::new(Mutex::new(HashMap::new()));
        let finished_tracks: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
        let abort = abort_option.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

        let prot_unlocked = lock_invariant(
            &prot,
            "player engine prot",
            "container selection and routing metadata must stay coherent",
        );
        let start_buffer_ms = lock_recoverable(
            &buffer_settings,
            "player engine buffer settings",
            "buffer settings are runtime configuration snapshots",
        )
        .start_buffer_ms;
        let channels = prot_unlocked.info.channels as usize;
        let _start_samples = ((prot_unlocked.info.sample_rate as f32 * start_buffer_ms) / 1000.0)
            as usize
            * channels;
        drop(prot_unlocked);

        Self {
            finished_tracks,
            start_time,
            buffer_map,
            buffer_notify,
            track_weights,
            track_channel_gains,
            effects_reset,
            inline_effects_update,
            inline_track_mix_updates,
            abort,
            prot,
            buffer_settings,
            effects,
            dsp_metrics,
            effect_settings_commands,
            mix_thread_handle: None,
        }
    }

    /// Start the output loop and invoke `f` for each mixed chunk.
    pub fn run_output_loop(&mut self, f: &dyn Fn((SamplesBuffer, f64))) {
        let prot = self.lock_prot_invariant();
        let keys = prot.get_keys();
        drop(prot);
        self.ready_buffer_map(&keys);
        let receiver = self.spawn_mix_receiver();

        for (mixer, length_in_seconds) in receiver {
            f((mixer, length_in_seconds));
        }
    }

    /// Legacy alias for [`Self::run_output_loop`].
    #[deprecated(note = "Use run_output_loop instead.")]
    pub fn reception_loop(&mut self, f: &dyn Fn((SamplesBuffer, f64))) {
        self.run_output_loop(f);
    }

    /// Start mixing and return a receiver for `(buffer, duration)` chunks.
    pub fn start_receiver(&mut self) -> Receiver<(SamplesBuffer, f64)> {
        let prot = self.lock_prot_invariant();
        let keys = prot.get_keys();
        drop(prot);
        self.ready_buffer_map(&keys);
        self.spawn_mix_receiver()
    }

    fn spawn_mix_receiver(&mut self) -> Receiver<(SamplesBuffer, f64)> {
        let prot = self.lock_prot_invariant();
        let audio_info = prot.info.clone();
        drop(prot);

        let (receiver, handle) = spawn_mix_thread(MixThreadArgs {
            audio_info,
            buffer_notify: self.buffer_notify.clone(),
            effects_reset: self.effects_reset.clone(),
            inline_effects_update: self.inline_effects_update.clone(),
            inline_track_mix_updates: self.inline_track_mix_updates.clone(),
            finished_tracks: self.finished_tracks.clone(),
            prot: self.prot.clone(),
            abort: self.abort.clone(),
            start_time: self.start_time,
            buffer_settings: self.buffer_settings.clone(),
            effects: self.effects.clone(),
            dsp_metrics: self.dsp_metrics.clone(),
            effect_settings_commands: self.effect_settings_commands.clone(),
        });
        self.mix_thread_handle = Some(handle);
        receiver
    }

    /// Get the total duration (seconds) of the active selection.
    pub fn get_duration(&self) -> f64 {
        let prot = self.lock_prot_invariant();
        *prot.get_duration()
    }

    /// Get finished engine track keys as a detached snapshot.
    pub fn finished_track_keys(&self) -> Vec<u16> {
        self.lock_finished_tracks_recoverable().clone()
    }

    fn ready_buffer_map(&mut self, keys: &[u32]) {
        self.buffer_map = init_buffer_map();
        self.lock_track_weights_recoverable().clear();
        self.lock_track_channel_gains_recoverable().clear();

        let prot = self.lock_prot_invariant();
        let sample_rate = prot.info.sample_rate;
        let channels = prot.info.channels as usize;
        let track_mix_settings = prot.get_track_mix_settings();
        let start_buffer_ms = self.lock_buffer_settings_recoverable().start_buffer_ms;
        drop(prot);
        let start_samples = ((sample_rate as f32 * start_buffer_ms) / 1000.0) as usize * channels;
        let buffer_size = (sample_rate as usize * 10).max(start_samples * 2);

        for key in keys {
            let Some(track_key) = u16::try_from(*key).ok() else {
                warn!(
                    "skipping track key {} because it exceeds engine key width (u16)",
                    key
                );
                continue;
            };
            let ring_buffer = Arc::new(Mutex::new(dasp_ring_buffer::Bounded::from(vec![
                    0.0;
                    buffer_size
                ])));
            self.lock_buffer_map_recoverable()
                .insert(track_key, ring_buffer);
            self.lock_track_weights_recoverable().insert(track_key, 1.0);
            let (level, pan) = track_mix_settings
                .get(&track_key)
                .copied()
                .unwrap_or((1.0, 0.0));
            let gains = compute_track_channel_gains(level, pan, channels);
            self.lock_track_channel_gains_recoverable()
                .insert(track_key, gains);
        }
    }

    /// Return true if all tracks have reported end-of-stream.
    pub fn finished_buffering(&self) -> bool {
        let finished_tracks = self.lock_finished_tracks_recoverable();
        let prot = self.lock_prot_invariant();
        let keys = prot.get_keys();
        drop(prot);

        for key in keys {
            let Some(track_key) = u16::try_from(key).ok() else {
                warn!(
                    "treating track key {} as unfinished because it exceeds engine key width (u16)",
                    key
                );
                return false;
            };
            if !finished_tracks.contains(&track_key) {
                return false;
            }
        }

        true
    }

    /// Invariant-only poison policy: engine container metadata must remain coherent.
    fn lock_prot_invariant(&self) -> MutexGuard<'_, Prot> {
        lock_invariant(
            &self.prot,
            "player engine prot",
            "container selection and routing metadata must stay coherent",
        )
    }

    /// Recoverable poison policy: engine buffer settings are runtime configuration snapshots.
    fn lock_buffer_settings_recoverable(&self) -> MutexGuard<'_, PlaybackBufferSettings> {
        lock_recoverable(
            &self.buffer_settings,
            "player engine buffer settings",
            "buffer settings are runtime configuration snapshots",
        )
    }

    /// Recoverable poison policy: finished-track bookkeeping is rebuildable runtime state.
    fn lock_finished_tracks_recoverable(&self) -> MutexGuard<'_, Vec<u16>> {
        lock_recoverable(
            &self.finished_tracks,
            "player engine finished tracks",
            "finished-track bookkeeping is rebuildable runtime state",
        )
    }

    /// Recoverable poison policy: the engine buffer map is rebuildable runtime state.
    fn lock_buffer_map_recoverable(&self) -> MutexGuard<'_, HashMap<u16, TrackBuffer>> {
        lock_recoverable(
            &self.buffer_map,
            "player engine buffer map",
            "per-track sample buffers are rebuildable runtime state",
        )
    }

    /// Recoverable poison policy: track weights are runtime mix configuration snapshots.
    fn lock_track_weights_recoverable(&self) -> MutexGuard<'_, HashMap<u16, f32>> {
        lock_recoverable(
            &self.track_weights,
            "player engine track weights",
            "track weights are runtime mix configuration snapshots",
        )
    }

    /// Recoverable poison policy: per-track channel gains are derived runtime mix state.
    fn lock_track_channel_gains_recoverable(&self) -> MutexGuard<'_, HashMap<u16, Vec<f32>>> {
        lock_recoverable(
            &self.track_channel_gains,
            "player engine track channel gains",
            "per-track channel gains are derived runtime mix state",
        )
    }
}

impl Drop for PlayerEngine {
    fn drop(&mut self) {
        self.abort.store(true, std::sync::atomic::Ordering::SeqCst);
        self.buffer_notify.notify_all();
        if let Some(handle) = self.mix_thread_handle.take() {
            if handle.join().is_err() {
                log::warn!("mix thread panicked during join");
            }
        }
    }
}

pub(crate) fn compute_track_channel_gains(level: f32, pan: f32, channels: usize) -> Vec<f32> {
    let level = level.max(0.0);
    if channels <= 1 {
        return vec![level];
    }

    let pan = pan.clamp(-1.0, 1.0);
    let left = if pan > 0.0 { 1.0 - pan } else { 1.0 };
    let right = if pan < 0.0 { 1.0 + pan } else { 1.0 };

    let mut gains = vec![level; channels];
    gains[0] = level * left;
    gains[1] = level * right;
    gains
}

#[cfg(test)]
mod tests {
    use super::compute_track_channel_gains;

    #[test]
    fn channel_gains_apply_level_and_pan() {
        let gains = compute_track_channel_gains(0.5, 0.5, 2);
        assert_eq!(gains.len(), 2);
        assert!((gains[0] - 0.25).abs() < 1e-6);
        assert!((gains[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mono_gain_uses_level_only() {
        let gains = compute_track_channel_gains(0.8, -1.0, 1);
        assert_eq!(gains, vec![0.8]);
    }
}
