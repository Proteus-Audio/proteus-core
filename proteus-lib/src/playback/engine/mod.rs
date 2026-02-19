//! Playback mixing engine and buffer coordination.

use dasp_ring_buffer::Bounded;
use rodio::buffer::SamplesBuffer;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc::Receiver, Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::audio::buffer::{init_buffer_map, TrackBuffer};
use crate::container::prot::Prot;

mod mix;
mod premix;
mod state;

pub use state::{DspChainMetrics, PlaybackBufferSettings};

use mix::{spawn_mix_thread, MixThreadArgs};

/// Request to update the active effects chain inline during playback.
#[derive(Debug, Clone)]
pub struct InlineEffectsUpdate {
    pub effects: Vec<crate::dsp::effects::AudioEffect>,
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
    pub slot_index: usize,
    pub level: f32,
    pub pan: f32,
}

/// Internal playback engine used by the high-level [`Player`].
#[derive(Debug)]
pub struct PlayerEngine {
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    start_time: f64,
    abort: Arc<AtomicBool>,
    buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    buffer_notify: Arc<Condvar>,
    effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    track_weights: Arc<Mutex<HashMap<u16, f32>>>,
    track_channel_gains: Arc<Mutex<HashMap<u16, Vec<f32>>>>,
    effects_reset: Arc<AtomicU64>,
    inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    prot: Arc<Mutex<Prot>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    effects: Arc<Mutex<Vec<crate::dsp::effects::AudioEffect>>>,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    mix_thread_handle: Option<JoinHandle<()>>,
}

impl PlayerEngine {
    /// Create a new engine for the given container and settings.
    pub fn new(
        prot: Arc<Mutex<Prot>>,
        abort_option: Option<Arc<AtomicBool>>,
        start_time: f64,
        buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
        effects: Arc<Mutex<Vec<crate::dsp::effects::AudioEffect>>>,
        dsp_metrics: Arc<Mutex<DspChainMetrics>>,
        effects_reset: Arc<AtomicU64>,
        inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
        inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    ) -> Self {
        let buffer_map = init_buffer_map();
        let buffer_notify = Arc::new(Condvar::new());
        let track_weights = Arc::new(Mutex::new(HashMap::new()));
        let track_channel_gains = Arc::new(Mutex::new(HashMap::new()));
        let finished_tracks: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
        let abort = if abort_option.is_some() {
            abort_option.unwrap()
        } else {
            Arc::new(AtomicBool::new(false))
        };

        let prot_unlocked = prot.lock().unwrap();
        let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
        let channels = prot_unlocked.info.channels as usize;
        let start_samples = ((prot_unlocked.info.sample_rate as f32 * start_buffer_ms) / 1000.0)
            as usize
            * channels;
        let buffer_size = (prot_unlocked.info.sample_rate as usize * 10).max(start_samples * 2);
        let effects_buffer = Arc::new(Mutex::new(dasp_ring_buffer::Bounded::from(vec![
            0.0;
            buffer_size
        ])));
        drop(prot_unlocked);

        Self {
            finished_tracks,
            start_time,
            buffer_map,
            buffer_notify,
            effects_buffer,
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
            mix_thread_handle: None,
        }
    }

    /// Start the mixing loop and invoke `f` for each mixed chunk.
    pub fn reception_loop(&mut self, f: &dyn Fn((SamplesBuffer, f64))) {
        let prot = self.prot.lock().unwrap();
        let keys = prot.get_keys();
        drop(prot);
        self.ready_buffer_map(&keys);
        let receiver = self.get_receiver();

        for (mixer, length_in_seconds) in receiver {
            f((mixer, length_in_seconds));
        }
    }

    /// Start mixing and return a receiver for `(buffer, duration)` chunks.
    pub fn start_receiver(&mut self) -> Receiver<(SamplesBuffer, f64)> {
        let prot = self.prot.lock().unwrap();
        let keys = prot.get_keys();
        drop(prot);
        self.ready_buffer_map(&keys);
        self.get_receiver()
    }

    fn get_receiver(&mut self) -> Receiver<(SamplesBuffer, f64)> {
        let prot = self.prot.lock().unwrap();
        let audio_info = prot.info.clone();
        drop(prot);

        let (receiver, handle) = spawn_mix_thread(MixThreadArgs {
            audio_info,
            buffer_map: self.buffer_map.clone(),
            buffer_notify: self.buffer_notify.clone(),
            effects_buffer: self.effects_buffer.clone(),
            track_weights: self.track_weights.clone(),
            track_channel_gains: self.track_channel_gains.clone(),
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
        });
        self.mix_thread_handle = Some(handle);
        receiver
    }

    /// Get the total duration (seconds) of the active selection.
    pub fn get_duration(&self) -> f64 {
        let prot = self.prot.lock().unwrap();
        *prot.get_duration()
    }

    fn ready_buffer_map(&mut self, keys: &Vec<u32>) {
        self.buffer_map = init_buffer_map();
        self.track_weights.lock().unwrap().clear();
        self.track_channel_gains.lock().unwrap().clear();

        let prot = self.prot.lock().unwrap();
        let sample_rate = prot.info.sample_rate;
        let channels = prot.info.channels as usize;
        let track_mix_settings = prot.get_track_mix_settings();
        let start_buffer_ms = self.buffer_settings.lock().unwrap().start_buffer_ms;
        drop(prot);
        let start_samples = ((sample_rate as f32 * start_buffer_ms) / 1000.0) as usize * channels;
        let buffer_size = (sample_rate as usize * 10).max(start_samples * 2);

        for key in keys {
            let ring_buffer = Arc::new(Mutex::new(dasp_ring_buffer::Bounded::from(vec![
                    0.0;
                    buffer_size
                ])));
            self.buffer_map
                .lock()
                .unwrap()
                .insert(*key as u16, ring_buffer);
            self.track_weights.lock().unwrap().insert(*key as u16, 1.0);
            let (level, pan) = track_mix_settings
                .get(&(*key as u16))
                .copied()
                .unwrap_or((1.0, 0.0));
            let gains = compute_track_channel_gains(level, pan, channels);
            self.track_channel_gains
                .lock()
                .unwrap()
                .insert(*key as u16, gains);
        }
    }

    /// Return true if all tracks have reported end-of-stream.
    pub fn finished_buffering(&self) -> bool {
        let finished_tracks = self.finished_tracks.lock().unwrap();
        let prot = self.prot.lock().unwrap();
        let keys = prot.get_keys();
        drop(prot);

        for key in keys {
            if !finished_tracks.contains(&(key as u16)) {
                return false;
            }
        }

        true
    }
}

impl Drop for PlayerEngine {
    fn drop(&mut self) {
        self.abort
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
