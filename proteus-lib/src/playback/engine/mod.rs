use dasp_ring_buffer::Bounded;
use rodio::buffer::SamplesBuffer;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc::Receiver, Arc, Condvar, Mutex};

use crate::audio::buffer::{init_buffer_map, TrackBuffer};
use crate::container::prot::Prot;

mod mix;
mod reverb;
mod state;

pub use state::{PlaybackBufferSettings, ReverbMetrics, ReverbSettings};

use mix::{spawn_mix_thread, MixThreadArgs};

#[derive(Debug, Clone)]
pub struct PlayerEngine {
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    start_time: f64,
    abort: Arc<AtomicBool>,
    buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    buffer_notify: Arc<Condvar>,
    effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    prot: Arc<Mutex<Prot>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
    reverb_metrics: Arc<Mutex<ReverbMetrics>>,
}

impl PlayerEngine {
    pub fn new(
        prot: Arc<Mutex<Prot>>,
        abort_option: Option<Arc<AtomicBool>>,
        start_time: f64,
        buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
        reverb_settings: Arc<Mutex<ReverbSettings>>,
        reverb_metrics: Arc<Mutex<ReverbMetrics>>,
    ) -> Self {
        let buffer_map = init_buffer_map();
        let buffer_notify = Arc::new(Condvar::new());
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
        let effects_buffer =
            Arc::new(Mutex::new(dasp_ring_buffer::Bounded::from(vec![0.0; buffer_size])));
        drop(prot_unlocked);

        Self {
            finished_tracks,
            start_time,
            buffer_map,
            buffer_notify,
            effects_buffer,
            abort,
            prot,
            buffer_settings,
            reverb_settings,
            reverb_metrics,
        }
    }

    pub fn reception_loop(&mut self, f: &dyn Fn((SamplesBuffer<f32>, f64))) {
        let prot = self.prot.lock().unwrap();
        let keys = prot.get_keys();
        drop(prot);
        self.ready_buffer_map(&keys);
        let receiver = self.get_receiver();

        for (mixer, length_in_seconds) in receiver {
            f((mixer, length_in_seconds));
        }
    }

    fn get_receiver(&self) -> Receiver<(SamplesBuffer<f32>, f64)> {
        let prot = self.prot.lock().unwrap();
        let audio_info = prot.info.clone();
        drop(prot);

        spawn_mix_thread(MixThreadArgs {
            audio_info,
            buffer_map: self.buffer_map.clone(),
            buffer_notify: self.buffer_notify.clone(),
            effects_buffer: self.effects_buffer.clone(),
            finished_tracks: self.finished_tracks.clone(),
            prot: self.prot.clone(),
            abort: self.abort.clone(),
            start_time: self.start_time,
            buffer_settings: self.buffer_settings.clone(),
            reverb_settings: self.reverb_settings.clone(),
            reverb_metrics: self.reverb_metrics.clone(),
        })
    }

    pub fn get_duration(&self) -> f64 {
        let prot = self.prot.lock().unwrap();
        *prot.get_duration()
    }

    fn ready_buffer_map(&mut self, keys: &Vec<u32>) {
        self.buffer_map = init_buffer_map();

        let prot = self.prot.lock().unwrap();
        let sample_rate = prot.info.sample_rate;
        let channels = prot.info.channels as usize;
        let start_buffer_ms = self.buffer_settings.lock().unwrap().start_buffer_ms;
        drop(prot);
        let start_samples = ((sample_rate as f32 * start_buffer_ms) / 1000.0) as usize * channels;
        let buffer_size = (sample_rate as usize * 1).max(start_samples * 2);

        for key in keys {
            let ring_buffer = Arc::new(Mutex::new(dasp_ring_buffer::Bounded::from(
                vec![0.0; buffer_size],
            )));
            self.buffer_map
                .lock()
                .unwrap()
                .insert(*key as u16, ring_buffer);
        }
    }

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
