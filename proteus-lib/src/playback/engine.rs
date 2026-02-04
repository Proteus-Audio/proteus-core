use dasp_ring_buffer::Bounded;
#[cfg(feature = "debug")]
use log::info;
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
#[cfg(feature = "debug")]
use std::time::Instant;
use std::{collections::HashMap, sync::mpsc::Receiver, thread};

use crate::{
    audio::buffer::*,
    container::prot::{ImpulseResponseSpec, Prot},
    dsp::impulse_response::{
        load_impulse_response_from_file_with_tail,
        load_impulse_response_from_prot_attachment_with_tail,
    },
    dsp::reverb::Reverb,
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
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
    reverb_metrics: Arc<Mutex<ReverbMetrics>>,
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

#[derive(Debug, Clone, Copy)]
pub struct PlaybackBufferSettings {
    pub start_buffer_ms: f32,
}

impl PlaybackBufferSettings {
    pub fn new(start_buffer_ms: f32) -> Self {
        Self {
            start_buffer_ms: start_buffer_ms.max(0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReverbMetrics {
    pub dsp_time_ms: f64,
    pub audio_time_ms: f64,
    pub rt_factor: f64,
    pub avg_dsp_ms: f64,
    pub avg_audio_ms: f64,
    pub avg_rt_factor: f64,
    pub min_rt_factor: f64,
    pub max_rt_factor: f64,
    pub buffer_fill: f64,
    pub avg_buffer_fill: f64,
    pub min_buffer_fill: f64,
    pub max_buffer_fill: f64,
    pub chain_time_ms: f64,
    pub avg_chain_time_ms: f64,
    pub min_chain_time_ms: f64,
    pub max_chain_time_ms: f64,
    pub out_interval_ms: f64,
    pub avg_out_interval_ms: f64,
    pub min_out_interval_ms: f64,
    pub max_out_interval_ms: f64,
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
        let effects_buffer = Arc::new(Mutex::new(Bounded::from(vec![0.0; buffer_size])));
        drop(prot_unlocked);

        let this = Self {
            finished_tracks,
            start_time,
            buffer_map,
            effects_buffer,
            abort,
            prot,
            buffer_settings,
            reverb_settings,
            reverb_metrics,
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
        let buffer_settings = self.buffer_settings.clone();
        let reverb_settings = self.reverb_settings.clone();
        let reverb_metrics = self.reverb_metrics.clone();

        thread::spawn(move || {
            #[cfg(feature = "debug")]
            let mut avg_buffer_fill = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut min_buffer_fill = f64::INFINITY;
            #[cfg(feature = "debug")]
            let mut max_buffer_fill = 0.0_f64;
            #[cfg(feature = "debug")]
            let alpha_buf = 0.1_f64;
            #[cfg(feature = "debug")]
            let mut last_send = Instant::now();
            #[cfg(feature = "debug")]
            let mut avg_chain_time_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut min_chain_time_ms = f64::INFINITY;
            #[cfg(feature = "debug")]
            let mut max_chain_time_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut avg_out_interval_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut min_out_interval_ms = f64::INFINITY;
            #[cfg(feature = "debug")]
            let mut max_out_interval_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let alpha_chain = 0.1_f64;

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

            let (impulse_spec, container_path, output_channels, tail_db) = {
                let prot = prot_locked.lock().unwrap();
                (
                    prot.get_impulse_response_spec(),
                    prot.get_container_path(),
                    prot.info.channels as usize,
                    prot.get_impulse_response_tail_db().unwrap_or(-60.0),
                )
            };

            let reverb = build_reverb_with_impulse_response(
                output_channels,
                0.000001,
                impulse_spec,
                container_path.as_deref(),
                tail_db,
            );
            let (reverb_sender, reverb_receiver) =
                spawn_reverb_worker(reverb, reverb_settings.clone(), reverb_metrics.clone());

            let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
            let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0)
                as usize
                * audio_info.channels as usize;
            let mut started = start_samples == 0;

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

                if !started {
                    let finished = finished_tracks.lock().unwrap();
                    let ready = hash_buffer.iter().all(|(track_key, buffer)| {
                        finished.contains(track_key) || buffer.len() >= start_samples
                    });
                    if ready {
                        started = true;
                    } else {
                        drop(hash_buffer);
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }

                if all_buffers_full
                    || (effects_buffer.lock().unwrap().len() > 0 && hash_buffer.len() == 0)
                {
                    #[cfg(feature = "debug")]
                    let chain_start = Instant::now();

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

                    let samples = process_reverb(
                        &reverb_sender,
                        &reverb_receiver,
                        vector_samples,
                        input_channels,
                        sample_rate,
                    );

                    let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

                    // Samples in the samples_buffer
                    let length_in_seconds = length_of_smallest_buffer as f64
                        / audio_info.sample_rate as f64
                        / audio_info.channels as f64;

                    #[cfg(feature = "debug")]
                    {
                        if started {
                            let total_samples = length_of_smallest_buffer as f64;
                            let capacity_samples = hash_buffer
                                .iter()
                                .map(|(_, buffer)| buffer.max_len())
                                .min()
                                .unwrap_or(0) as f64;
                            let fill = if capacity_samples > 0.0 {
                                total_samples / capacity_samples
                            } else {
                                0.0
                            };

                            avg_buffer_fill = if avg_buffer_fill == 0.0 {
                                fill
                            } else {
                                (avg_buffer_fill * (1.0 - alpha_buf)) + (fill * alpha_buf)
                            };
                            min_buffer_fill = min_buffer_fill.min(fill);
                            max_buffer_fill = max_buffer_fill.max(fill);

                            let mut metrics = reverb_metrics.lock().unwrap();
                            metrics.buffer_fill = fill;
                            metrics.avg_buffer_fill = avg_buffer_fill;
                            metrics.min_buffer_fill = if min_buffer_fill.is_finite() {
                                min_buffer_fill
                            } else {
                                0.0
                            };
                            metrics.max_buffer_fill = max_buffer_fill;
                        }
                    }

                    #[cfg(feature = "debug")]
                    {
                        let chain_time_ms = chain_start.elapsed().as_secs_f64() * 1000.0;
                        let out_interval_ms = last_send.elapsed().as_secs_f64() * 1000.0;

                        avg_chain_time_ms = if avg_chain_time_ms == 0.0 {
                            chain_time_ms
                        } else {
                            (avg_chain_time_ms * (1.0 - alpha_chain))
                                + (chain_time_ms * alpha_chain)
                        };
                        min_chain_time_ms = min_chain_time_ms.min(chain_time_ms);
                        max_chain_time_ms = max_chain_time_ms.max(chain_time_ms);

                        avg_out_interval_ms = if avg_out_interval_ms == 0.0 {
                            out_interval_ms
                        } else {
                            (avg_out_interval_ms * (1.0 - alpha_chain))
                                + (out_interval_ms * alpha_chain)
                        };
                        min_out_interval_ms = min_out_interval_ms.min(out_interval_ms);
                        max_out_interval_ms = max_out_interval_ms.max(out_interval_ms);

                        let mut metrics = reverb_metrics.lock().unwrap();
                        metrics.chain_time_ms = chain_time_ms;
                        metrics.avg_chain_time_ms = avg_chain_time_ms;
                        metrics.min_chain_time_ms = if min_chain_time_ms.is_finite() {
                            min_chain_time_ms
                        } else {
                            0.0
                        };
                        metrics.max_chain_time_ms = max_chain_time_ms;
                        metrics.out_interval_ms = out_interval_ms;
                        metrics.avg_out_interval_ms = avg_out_interval_ms;
                        metrics.min_out_interval_ms = if min_out_interval_ms.is_finite() {
                            min_out_interval_ms
                        } else {
                            0.0
                        };
                        metrics.max_out_interval_ms = max_out_interval_ms;
                    }

                    sender.send((samples_buffer, length_in_seconds)).unwrap();
                    #[cfg(feature = "debug")]
                    {
                        last_send = Instant::now();
                    }
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
        let channels = prot.info.channels as usize;
        let start_buffer_ms = self.buffer_settings.lock().unwrap().start_buffer_ms;
        drop(prot);
        let start_samples =
            ((sample_rate as f32 * start_buffer_ms) / 1000.0) as usize * channels;
        let buffer_size = (sample_rate as usize * 1).max(start_samples * 2);

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
    tail_db: f32,
) -> Reverb {
    let impulse_spec = match impulse_spec {
        Some(spec) => spec,
        None => return Reverb::new(channels, dry_wet),
    };

    let result = match impulse_spec {
        ImpulseResponseSpec::Attachment(name) => container_path
            .ok_or_else(|| "missing container path for attachment".to_string())
            .and_then(|path| {
                load_impulse_response_from_prot_attachment_with_tail(path, &name, Some(tail_db))
                    .map_err(|err| err.to_string())
            }),
        ImpulseResponseSpec::FilePath(path) => {
            let resolved_path = resolve_impulse_response_path(container_path, &path);
            if resolved_path.exists() {
                load_impulse_response_from_file_with_tail(&resolved_path, Some(tail_db))
                    .map_err(|err| err.to_string())
            } else {
                match container_path {
                    Some(container_path) => {
                        let fallback_name = Path::new(&path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.to_string());
                        if let Some(fallback_name) = fallback_name {
                            load_impulse_response_from_prot_attachment_with_tail(
                                container_path,
                                &fallback_name,
                                Some(tail_db),
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
    channels: u16,
    sample_rate: u32,
}

struct ReverbResult {
    samples: Vec<f32>,
}

fn spawn_reverb_worker(
    mut reverb: Reverb,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
    reverb_metrics: Arc<Mutex<ReverbMetrics>>,
) -> (mpsc::SyncSender<ReverbJob>, mpsc::Receiver<ReverbResult>) {
    let (job_sender, job_receiver) = mpsc::sync_channel::<ReverbJob>(1);
    let (result_sender, result_receiver) = mpsc::sync_channel::<ReverbResult>(1);

    thread::spawn(move || {
            let mut current_dry_wet = 0.000001_f32;
            #[cfg(feature = "debug")]
            let mut last_log = Instant::now();
            #[cfg(feature = "debug")]
            let mut avg_dsp_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut avg_audio_ms = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut avg_rt_factor = 0.0_f64;
            #[cfg(feature = "debug")]
            let mut min_rt_factor = f64::INFINITY;
            #[cfg(feature = "debug")]
            let mut max_rt_factor = 0.0_f64;
            #[cfg(feature = "debug")]
            let alpha = 0.1_f64;

            #[cfg(not(feature = "debug"))]
            let _ = &reverb_metrics;

        while let Ok(job) = job_receiver.recv() {
            let settings = *reverb_settings.lock().unwrap();
            if (settings.dry_wet - current_dry_wet).abs() > f32::EPSILON {
                reverb.set_dry_wet(settings.dry_wet);
                current_dry_wet = settings.dry_wet;
            }

            #[cfg(feature = "debug")]
            let audio_time_ms = if job.channels > 0 && job.sample_rate > 0 {
                let frames = job.samples.len() as f64 / job.channels as f64;
                (frames / job.sample_rate as f64) * 1000.0
            } else {
                0.0
            };

            #[cfg(feature = "debug")]
            let dsp_start = Instant::now();
            let samples = if settings.enabled && settings.dry_wet > 0.0 {
                reverb.process(&job.samples)
            } else {
                job.samples
            };
            #[cfg(feature = "debug")]
            let dsp_time_ms = dsp_start.elapsed().as_secs_f64() * 1000.0;
            #[cfg(feature = "debug")]
            let rt_factor = if audio_time_ms > 0.0 {
                dsp_time_ms / audio_time_ms
            } else {
                0.0
            };

            #[cfg(feature = "debug")]
            {
                avg_dsp_ms = if avg_dsp_ms == 0.0 {
                    dsp_time_ms
                } else {
                    (avg_dsp_ms * (1.0 - alpha)) + (dsp_time_ms * alpha)
                };
                avg_audio_ms = if avg_audio_ms == 0.0 {
                    audio_time_ms
                } else {
                    (avg_audio_ms * (1.0 - alpha)) + (audio_time_ms * alpha)
                };
                avg_rt_factor = if avg_rt_factor == 0.0 {
                    rt_factor
                } else {
                    (avg_rt_factor * (1.0 - alpha)) + (rt_factor * alpha)
                };

                if rt_factor > 0.0 {
                    if rt_factor < min_rt_factor {
                        min_rt_factor = rt_factor;
                    }
                    if rt_factor > max_rt_factor {
                        max_rt_factor = rt_factor;
                    }
                }

                let mut metrics = reverb_metrics.lock().unwrap();
                metrics.dsp_time_ms = dsp_time_ms;
                metrics.audio_time_ms = audio_time_ms;
                metrics.rt_factor = rt_factor;
                metrics.avg_dsp_ms = avg_dsp_ms;
                metrics.avg_audio_ms = avg_audio_ms;
                metrics.avg_rt_factor = avg_rt_factor;
                metrics.min_rt_factor = if min_rt_factor.is_finite() {
                    min_rt_factor
                } else {
                    0.0
                };
                metrics.max_rt_factor = max_rt_factor;
            }

            #[cfg(feature = "debug")]
            if last_log.elapsed().as_secs_f64() >= 1.0 {
                info!(
                    "DSP packet: {:.2}ms / {:.2}ms ({:.2}x realtime) avg {:.2}x (min {:.2}x max {:.2}x)",
                    dsp_time_ms,
                    audio_time_ms,
                    rt_factor,
                    avg_rt_factor,
                    if min_rt_factor.is_finite() {
                        min_rt_factor
                    } else {
                        0.0
                    },
                    max_rt_factor
                );
                last_log = Instant::now();
            }

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
    channels: u16,
    sample_rate: u32,
) -> Vec<f32> {
    if sender
        .send(ReverbJob {
            samples,
            channels,
            sample_rate,
        })
        .is_err()
    {
        return Vec::new();
    }

    match receiver.recv() {
        Ok(result) => result.samples,
        Err(_) => Vec::new(),
    }
}
