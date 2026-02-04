use dasp_ring_buffer::Bounded;
use log::warn;
use rodio::{
    buffer::SamplesBuffer,
    dynamic_mixer::{self},
    Source,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::{collections::HashMap, sync::mpsc::Receiver, thread};

use crate::{
    buffer::*,
    effects::{
        impulse_response::{
            load_impulse_response_from_file, load_impulse_response_from_prot_attachment,
        },
        reverb::Reverb,
    },
    prot::{ImpulseResponseSpec, Prot},
};
// use crate::effects::*;
use crate::track::*;

#[derive(Debug, Clone)]
pub struct PlayerEngine {
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    start_time: f64,
    abort: Arc<AtomicBool>,
    buffer_map: Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    prot: Arc<Mutex<Prot>>,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ReverbSettings {
    pub enabled: bool,
    pub dry_wet: f32,
}

impl ReverbSettings {
    pub fn new(dry_wet: f32) -> Self {
        Self {
            enabled: true,
            dry_wet: dry_wet.clamp(0.0, 1.0),
        }
    }
}

impl PlayerEngine {
    pub fn new(
        prot: Arc<Mutex<Prot>>,
        abort_option: Option<Arc<AtomicBool>>,
        start_time: f64,
        reverb_settings: Arc<Mutex<ReverbSettings>>,
    ) -> Self {
        let buffer_map = init_buffer_map();
        let finished_tracks: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
        let abort = if abort_option.is_some() {
            abort_option.unwrap()
        } else {
            Arc::new(AtomicBool::new(false))
        };

        let prot_unlocked = prot.lock().unwrap();
        let buffer_size = prot_unlocked.info.sample_rate as usize * 10; // Ten seconds of audio at the sample rate
        let effects_buffer = Arc::new(Mutex::new(Bounded::from(vec![0.0; buffer_size])));
        drop(prot_unlocked);

        let this = Self {
            finished_tracks,
            start_time,
            buffer_map,
            effects_buffer,
            abort,
            prot,
            reverb_settings,
        };

        this
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
        // let (sender, receiver) = mpsc::sync_channel::<(DynamicMixer<f32>, f64)>(1);
        let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer<f32>, f64)>(1);

        let prot = self.prot.lock().unwrap();
        let audio_info = prot.info.clone();
        drop(prot);
        let buffer_map = self.buffer_map.clone();
        let abort = self.abort.clone();

        let finished_tracks = self.finished_tracks.clone();
        let effects_buffer = self.effects_buffer.clone();
        let prot_locked = self.prot.clone();
        let start_time = self.start_time;
        let reverb_settings = self.reverb_settings.clone();

        thread::spawn(move || {
            let prot = prot_locked.lock().unwrap();
            let enumerated_list = prot.enumerated_list();
            drop(prot);

            for (key, file_path, track_id) in enumerated_list {
                buffer_track(
                    TrackArgs {
                        file_path: file_path.clone(),
                        track_id,
                        track_key: key,
                        buffer_map: buffer_map.clone(),
                        finished_tracks: finished_tracks.clone(),
                        start_time,
                        channels: audio_info.channels as u8,
                    },
                    abort.clone(),
                );
            }

            // let sink_mutex_copy = sink_mutex.clone();
            let hash_buffer_copy = buffer_map.clone();

            let (impulse_spec, container_path, output_channels) = {
                let prot = prot_locked.lock().unwrap();
                (
                    prot.get_impulse_response_spec(),
                    prot.get_container_path(),
                    prot.info.channels as usize,
                )
            };

            let reverb = build_reverb_with_impulse_response(
                output_channels,
                0.000001,
                impulse_spec,
                container_path.as_deref(),
            );
            let (reverb_sender, reverb_receiver) =
                spawn_reverb_worker(reverb, reverb_settings.clone());

            loop {
                if abort.load(Ordering::SeqCst) {
                    break;
                }

                let mut hash_buffer = hash_buffer_copy.lock().unwrap();

                let mut removable_tracks: Vec<u16> = Vec::new();

                // if all buffers are not empty, add samples from each buffer to the mixer
                // until at least one buffer is empty
                let mut all_buffers_full = true;
                for (track_key, buffer) in hash_buffer.iter() {
                    if buffer.len() == 0 {
                        let finished = finished_tracks.lock().unwrap();
                        if finished.contains(&track_key) {
                            removable_tracks.push(*track_key);
                            continue;
                        }
                        all_buffers_full = false;
                    }
                }

                for track_id in removable_tracks {
                    hash_buffer.remove(&track_id);
                }

                // If hash_buffer contains no tracks, exit the loop
                if hash_buffer.len() == 0 && effects_buffer.lock().unwrap().len() == 0 {
                    break;
                }

                if all_buffers_full
                    || (effects_buffer.lock().unwrap().len() > 0 && hash_buffer.len() == 0)
                {
                    let (controller, mixer) = dynamic_mixer::mixer::<f32>(
                        audio_info.channels as u16,
                        audio_info.sample_rate as u32,
                    );

                    // Hash buffer plus effects buffer
                    let mut effects_buffer_unlocked = effects_buffer.lock().unwrap();
                    let mut combined_buffer: HashMap<u16, Bounded<Vec<f32>>> = HashMap::new();
                    for (track_key, buffer) in hash_buffer.iter() {
                        combined_buffer.insert(*track_key, buffer.clone());
                    }

                    let length_of_smallest_buffer = hash_buffer
                        .iter()
                        .map(|(_, buffer)| buffer.len())
                        .min()
                        .unwrap();
                    for (_, buffer) in hash_buffer.iter_mut() {
                        let mut samples: Vec<f32> = Vec::new();
                        for _ in 0..length_of_smallest_buffer {
                            samples.push(buffer.pop().unwrap());
                        }

                        let source = SamplesBuffer::new(
                            audio_info.channels as u16,
                            audio_info.sample_rate as u32,
                            samples,
                        );

                        controller.add(source.convert_samples().amplify(0.2));
                    }

                    // Add effects buffer to mixer
                    let num_effects_samples =
                        if effects_buffer_unlocked.len() < length_of_smallest_buffer {
                            effects_buffer_unlocked.len()
                        } else {
                            length_of_smallest_buffer
                        };

                    {
                        let mut samples: Vec<f32> = Vec::new();
                        for _ in 0..num_effects_samples {
                            samples.push(effects_buffer_unlocked.pop().unwrap());
                        }

                        let source = SamplesBuffer::new(
                            audio_info.channels as u16,
                            audio_info.sample_rate as u32,
                            samples,
                        );

                        controller.add(source.convert_samples().amplify(0.2));
                    }

                    drop(effects_buffer_unlocked);

                    let sample_rate = mixer.sample_rate();
                    let mixer_buffered = mixer.buffered();
                    let vector_samples = mixer_buffered.clone().into_iter().collect::<Vec<f32>>();
                    let input_channels = mixer_buffered.channels();

                    let samples = process_reverb(&reverb_sender, &reverb_receiver, vector_samples);

                    let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

                    // Samples in the samples_buffer
                    let length_in_seconds = length_of_smallest_buffer as f64
                        / audio_info.sample_rate as f64
                        / audio_info.channels as f64;

                    sender.send((samples_buffer, length_in_seconds)).unwrap();
                }

                drop(hash_buffer);

                thread::sleep(Duration::from_millis(100));
            }
        });

        // Arc::new(receiver)
        receiver
    }

    pub fn get_duration(&self) -> f64 {
        let prot = self.prot.lock().unwrap();
        *prot.get_duration()
    }

    fn ready_buffer_map(&mut self, keys: &Vec<u32>) {
        self.buffer_map = init_buffer_map();

        let prot = self.prot.lock().unwrap();
        let sample_rate = prot.info.sample_rate;
        drop(prot);
        let buffer_size = sample_rate as usize * 1; // Ten seconds of audio at the sample rate

        for key in keys {
            let ring_buffer = Bounded::from(vec![0.0; buffer_size]);
            self.buffer_map
                .lock()
                .unwrap()
                .insert(*key as u16, ring_buffer);
        }
    }

    // pub fn abort(&self) {
    //     self.abort.store(true, Ordering::SeqCst);
    // }

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

fn build_reverb_with_impulse_response(
    channels: usize,
    dry_wet: f32,
    impulse_spec: Option<ImpulseResponseSpec>,
    container_path: Option<&str>,
) -> Reverb {
    let impulse_spec = match impulse_spec {
        Some(spec) => spec,
        None => return Reverb::new(channels, dry_wet),
    };

    let result = match impulse_spec {
        ImpulseResponseSpec::Attachment(name) => container_path
            .ok_or_else(|| "missing container path for attachment".to_string())
            .and_then(|path| {
                load_impulse_response_from_prot_attachment(path, &name)
                    .map_err(|err| err.to_string())
            }),
        ImpulseResponseSpec::FilePath(path) => {
            let resolved_path = resolve_impulse_response_path(container_path, &path);
            if resolved_path.exists() {
                load_impulse_response_from_file(&resolved_path).map_err(|err| err.to_string())
            } else {
                match container_path {
                    Some(container_path) => {
                        let fallback_name = Path::new(&path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.to_string());
                        if let Some(fallback_name) = fallback_name {
                            load_impulse_response_from_prot_attachment(
                                container_path,
                                &fallback_name,
                            )
                            .map_err(|err| err.to_string())
                        } else {
                            Err(format!(
                                "impulse response path not found: {}",
                                resolved_path.display()
                            ))
                        }
                    }
                    None => Err(format!(
                        "impulse response path not found: {}",
                        resolved_path.display()
                    )),
                }
            }
        }
    };

    match result {
        Ok(impulse_response) => {
            Reverb::new_with_impulse_response(channels, dry_wet, &impulse_response)
        }
        Err(err) => {
            warn!(
                "Failed to load impulse response ({}); falling back to default reverb.",
                err
            );
            Reverb::new(channels, dry_wet)
        }
    }
}

fn resolve_impulse_response_path(container_path: Option<&str>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(container_path) = container_path {
        if let Some(parent) = Path::new(container_path).parent() {
            return parent.join(path);
        }
    }

    path.to_path_buf()
}

struct ReverbJob {
    samples: Vec<f32>,
}

struct ReverbResult {
    samples: Vec<f32>,
}

fn spawn_reverb_worker(
    mut reverb: Reverb,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
) -> (mpsc::SyncSender<ReverbJob>, mpsc::Receiver<ReverbResult>) {
    let (job_sender, job_receiver) = mpsc::sync_channel::<ReverbJob>(1);
    let (result_sender, result_receiver) = mpsc::sync_channel::<ReverbResult>(1);

    thread::spawn(move || {
        let mut current_dry_wet = 0.000001_f32;

        while let Ok(job) = job_receiver.recv() {
            let settings = *reverb_settings.lock().unwrap();
            if (settings.dry_wet - current_dry_wet).abs() > f32::EPSILON {
                reverb.set_dry_wet(settings.dry_wet);
                current_dry_wet = settings.dry_wet;
            }

            let samples = if settings.enabled && settings.dry_wet > 0.0 {
                reverb.process(&job.samples)
            } else {
                job.samples
            };

            if result_sender.send(ReverbResult { samples }).is_err() {
                break;
            }
        }
    });

    (job_sender, result_receiver)
}

fn process_reverb(
    sender: &mpsc::SyncSender<ReverbJob>,
    receiver: &mpsc::Receiver<ReverbResult>,
    samples: Vec<f32>,
) -> Vec<f32> {
    if sender.send(ReverbJob { samples }).is_err() {
        return Vec::new();
    }

    match receiver.recv() {
        Ok(result) => result.samples,
        Err(_) => Vec::new(),
    }
}
