use dasp_ring_buffer::Bounded;
use rodio::{
    buffer::SamplesBuffer,
    dynamic_mixer::{self},
    Source,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
#[cfg(feature = "debug")]
use std::time::Instant;

use crate::container::prot::Prot;
use crate::track::{buffer_container_tracks, buffer_track, ContainerTrackArgs, TrackArgs};

use super::reverb::{build_reverb_with_impulse_response, process_reverb, spawn_reverb_worker};
use super::state::{PlaybackBufferSettings, ReverbMetrics, ReverbSettings};

pub struct MixThreadArgs {
    pub audio_info: crate::container::info::Info,
    pub buffer_map: Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    pub buffer_notify: Arc<std::sync::Condvar>,
    pub effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub prot: Arc<Mutex<Prot>>,
    pub abort: Arc<AtomicBool>,
    pub start_time: f64,
    pub buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub reverb_settings: Arc<Mutex<ReverbSettings>>,
    pub reverb_metrics: Arc<Mutex<ReverbMetrics>>,
}

pub fn spawn_mix_thread(args: MixThreadArgs) -> mpsc::Receiver<(SamplesBuffer<f32>, f64)> {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer<f32>, f64)>(1);

    let MixThreadArgs {
        audio_info,
        buffer_map,
        buffer_notify,
        effects_buffer,
        finished_tracks,
        prot,
        abort,
        start_time,
        buffer_settings,
        reverb_settings,
        reverb_metrics,
    } = args;

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

        let prot_locked = prot.clone();

        let prot = prot_locked.lock().unwrap();
        let container_tracks = prot.container_track_entries();
        let enumerated_list = if container_tracks.is_some() {
            Vec::new()
        } else {
            prot.enumerated_list()
        };
        drop(prot);

        if let Some((file_path, track_entries)) = container_tracks {
            buffer_container_tracks(
                ContainerTrackArgs {
                    file_path,
                    track_entries,
                    buffer_map: buffer_map.clone(),
                    buffer_notify: Some(buffer_notify.clone()),
                    finished_tracks: finished_tracks.clone(),
                    start_time,
                    channels: audio_info.channels as u8,
                },
                abort.clone(),
            );
        } else {
            for (key, file_path, track_id) in enumerated_list {
                buffer_track(
                    TrackArgs {
                        file_path: file_path.clone(),
                        track_id,
                        track_key: key,
                        buffer_map: buffer_map.clone(),
                        buffer_notify: Some(buffer_notify.clone()),
                        finished_tracks: finished_tracks.clone(),
                        start_time,
                        channels: audio_info.channels as u8,
                    },
                    abort.clone(),
                );
            }
        }

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
        let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0) as usize
            * audio_info.channels as usize;
        let mut started = start_samples == 0;

        loop {
            if abort.load(Ordering::SeqCst) {
                break;
            }

            let mut hash_buffer = hash_buffer_copy.lock().unwrap();

            let mut removable_tracks: Vec<u16> = Vec::new();

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
                        let (guard, _) = buffer_notify
                            .wait_timeout(hash_buffer, Duration::from_millis(20))
                            .unwrap();
                        drop(guard);
                        continue;
                    }
                }

                let effects_len = effects_buffer.lock().unwrap().len();
                if all_buffers_full || (effects_len > 0 && hash_buffer.len() == 0) {
                    #[cfg(feature = "debug")]
                    let chain_start = Instant::now();

                let (controller, mixer) = dynamic_mixer::mixer::<f32>(
                    audio_info.channels as u16,
                    audio_info.sample_rate as u32,
                );

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

                let num_effects_samples = if effects_buffer_unlocked.len() < length_of_smallest_buffer
                {
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

            if !all_buffers_full && effects_len == 0 {
                let (guard, _) = buffer_notify
                    .wait_timeout(hash_buffer, Duration::from_millis(20))
                    .unwrap();
                drop(guard);
            } else {
                drop(hash_buffer);
            }
        }
    });

    receiver
}
