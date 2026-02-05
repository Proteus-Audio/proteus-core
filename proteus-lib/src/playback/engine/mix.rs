use dasp_ring_buffer::Bounded;
use rodio::buffer::SamplesBuffer;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
#[cfg(feature = "debug")]
use std::time::Instant;

use crate::audio::buffer::TrackBuffer;
use crate::container::prot::Prot;
use crate::track::{buffer_container_tracks, buffer_track, ContainerTrackArgs, TrackArgs};

use super::reverb::build_reverb_with_impulse_response;
use super::state::{PlaybackBufferSettings, ReverbMetrics, ReverbSettings};

pub struct MixThreadArgs {
    pub audio_info: crate::container::info::Info,
    pub buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    pub buffer_notify: Arc<std::sync::Condvar>,
    pub effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    pub track_weights: Arc<Mutex<HashMap<u16, f32>>>,
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
        track_weights,
        finished_tracks,
        prot,
        abort,
        start_time,
        buffer_settings,
        reverb_settings,
        reverb_metrics,
    } = args;

    thread::spawn(move || {
        const MIN_MIX_MS: f32 = 300.0;
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
        #[cfg(feature = "debug")]
        let mut wake_total: u64 = 0;
        #[cfg(feature = "debug")]
        let mut wake_idle: u64 = 0;

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
                    track_weights: Some(track_weights.clone()),
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
                        track_weights: None,
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

        let mut reverb = build_reverb_with_impulse_response(
            output_channels,
            0.000001,
            impulse_spec,
            container_path.as_deref(),
            tail_db,
        );
        let mut current_dry_wet = 0.000001_f32;
        let mut reverb_input_buf: Vec<f32> = Vec::new();
        let mut reverb_output_buf: Vec<f32> = Vec::new();
        let reverb_block_samples = reverb.block_size_samples();
        const REVERB_BATCH_BLOCKS: usize = 2;
        let mut reverb_block_out: Vec<f32> = Vec::new();
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

        let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
        let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0) as usize
            * audio_info.channels as usize;
        let min_mix_frames = ((audio_info.sample_rate as f32 * MIN_MIX_MS) / 1000.0) as usize;
        let min_mix_samples = min_mix_frames.max(1) * audio_info.channels as usize;
        let mut started = start_samples == 0;
        let mut mix_buffer = vec![0.0_f32; min_mix_samples];

        loop {
            if abort.load(Ordering::SeqCst) {
                break;
            }

            let buffer_snapshot: Vec<(u16, TrackBuffer)> = {
                let map = hash_buffer_copy.lock().unwrap();
                map.iter().map(|(k, v)| (*k, v.clone())).collect()
            };
            let weights_snapshot: HashMap<u16, f32> = {
                let weights = track_weights.lock().unwrap();
                weights.clone()
            };
            let mut removable_tracks: Vec<u16> = Vec::new();
            #[cfg(feature = "debug")]
            {
                wake_total = wake_total.saturating_add(1);
            }

            let mut all_buffers_full = true;
            for (track_key, buffer) in buffer_snapshot.iter() {
                let len = buffer.lock().unwrap().len();
                if len == 0 {
                    let finished = finished_tracks.lock().unwrap();
                    if finished.contains(track_key) {
                        removable_tracks.push(*track_key);
                        continue;
                    }
                    all_buffers_full = false;
                }
            }

            if !removable_tracks.is_empty() {
                let mut map = hash_buffer_copy.lock().unwrap();
                for track_id in removable_tracks.drain(..) {
                    map.remove(&track_id);
                }
            }

            if buffer_snapshot.is_empty() && effects_buffer.lock().unwrap().len() == 0 {
                break;
            }

            if !started {
                let finished = finished_tracks.lock().unwrap();
                let ready = buffer_snapshot.iter().all(|(track_key, buffer)| {
                    let len = buffer.lock().unwrap().len();
                    finished.contains(track_key) || len >= start_samples
                });
                if ready {
                    started = true;
                } else {
                    #[cfg(feature = "debug")]
                    {
                        wake_idle = wake_idle.saturating_add(1);
                        let mut metrics = reverb_metrics.lock().unwrap();
                        metrics.wake_total = wake_total;
                        metrics.wake_idle = wake_idle;
                    }
                    let (guard, _) = buffer_notify
                        .wait_timeout(hash_buffer_copy.lock().unwrap(), Duration::from_millis(20))
                        .unwrap();
                    drop(guard);
                    continue;
                }
            }

            let effects_len = effects_buffer.lock().unwrap().len();
            let mut did_work = false;
            let min_len = if buffer_snapshot.is_empty() {
                0
            } else {
                buffer_snapshot
                    .iter()
                    .map(|(_, buffer)| buffer.lock().unwrap().len())
                    .min()
                    .unwrap_or(0)
            };
            let all_tracks_finished = {
                let finished = finished_tracks.lock().unwrap();
                buffer_snapshot
                    .iter()
                    .all(|(track_key, _)| finished.contains(track_key))
            };
            let should_mix_tracks = !buffer_snapshot.is_empty()
                && (min_len >= min_mix_samples || (all_tracks_finished && min_len > 0));
            let should_mix_effects = buffer_snapshot.is_empty()
                && (effects_len >= min_mix_samples || (all_tracks_finished && effects_len > 0));

            if should_mix_tracks || should_mix_effects {
                #[cfg(feature = "debug")]
                let chain_start = Instant::now();

                let mut effects_buffer_unlocked = effects_buffer.lock().unwrap();
                let length_of_smallest_buffer = if buffer_snapshot.is_empty() {
                    0
                } else {
                    min_len
                };
                let current_chunk = if !buffer_snapshot.is_empty() {
                    if length_of_smallest_buffer >= min_mix_samples {
                        min_mix_samples
                    } else if all_tracks_finished && length_of_smallest_buffer > 0 {
                        length_of_smallest_buffer
                    } else {
                        0
                    }
                } else if effects_buffer_unlocked.len() >= min_mix_samples {
                    min_mix_samples
                } else if all_tracks_finished && effects_buffer_unlocked.len() > 0 {
                    effects_buffer_unlocked.len()
                } else {
                    0
                };

                if current_chunk == 0 {
                    drop(effects_buffer_unlocked);
                    if !all_buffers_full && effects_len == 0 {
                        #[cfg(feature = "debug")]
                        if !did_work {
                            wake_idle = wake_idle.saturating_add(1);
                            let mut metrics = reverb_metrics.lock().unwrap();
                            metrics.wake_total = wake_total;
                            metrics.wake_idle = wake_idle;
                        }
                        let (guard, _) = buffer_notify
                            .wait_timeout(
                                hash_buffer_copy.lock().unwrap(),
                                Duration::from_millis(20),
                            )
                            .unwrap();
                        drop(guard);
                    } else {
                        drop(buffer_snapshot);
                    }
                    continue;
                }

                mix_buffer.fill(0.0);

                if !buffer_snapshot.is_empty() {
                    for (track_key, buffer) in buffer_snapshot.iter() {
                        let weight = weights_snapshot.get(track_key).copied().unwrap_or(1.0);
                        let mut buffer = buffer.lock().unwrap();
                        for sample in mix_buffer.iter_mut().take(current_chunk) {
                            *sample += buffer.pop().unwrap() * weight;
                        }
                    }
                }

                if effects_buffer_unlocked.len() > 0 {
                    let effects_take = effects_buffer_unlocked.len().min(current_chunk);
                    for sample in mix_buffer.iter_mut().take(effects_take) {
                        *sample += effects_buffer_unlocked.pop().unwrap();
                    }
                }

                drop(effects_buffer_unlocked);

                let input_channels = audio_info.channels as u16;
                let sample_rate = audio_info.sample_rate as u32;
                let mut settings = reverb_settings.lock().unwrap();
                if settings.reset_pending {
                    reverb.clear_state();
                    reverb_input_buf.clear();
                    reverb_output_buf.clear();
                    reverb_block_out.clear();
                    settings.reset_pending = false;
                }
                let settings = *settings;
                if (settings.dry_wet - current_dry_wet).abs() > f32::EPSILON {
                    reverb.set_dry_wet(settings.dry_wet);
                    current_dry_wet = settings.dry_wet;
                }

                #[cfg(feature = "debug")]
                let audio_time_ms = if input_channels > 0 && sample_rate > 0 {
                    let frames = current_chunk as f64 / input_channels as f64;
                    (frames / sample_rate as f64) * 1000.0
                } else {
                    0.0
                };

                #[cfg(feature = "debug")]
                let dsp_start = Instant::now();
                let samples = if settings.enabled && settings.dry_wet > 0.0 {
                    if reverb_block_samples == 0 {
                        reverb.process(&mix_buffer[..current_chunk])
                    } else {
                        reverb_input_buf.extend_from_slice(&mix_buffer[..current_chunk]);
                        let batch_samples = reverb_block_samples * REVERB_BATCH_BLOCKS;
                        let should_flush = all_tracks_finished && !reverb_input_buf.is_empty();
                        while reverb_input_buf.len() >= batch_samples || should_flush {
                            let take = if reverb_input_buf.len() >= batch_samples {
                                batch_samples
                            } else {
                                reverb_input_buf.len()
                            };
                            let block: Vec<f32> = reverb_input_buf.drain(0..take).collect();
                            reverb.process_into(&block, &mut reverb_block_out);
                            reverb_output_buf.extend_from_slice(&reverb_block_out);
                            if take < batch_samples {
                                break;
                            }
                        }
                        if reverb_output_buf.len() < current_chunk {
                            let mut out = reverb_output_buf.clone();
                            out.extend(std::iter::repeat(0.0).take(current_chunk - out.len()));
                            reverb_output_buf.clear();
                            out
                        } else {
                            reverb_output_buf.drain(0..current_chunk).collect()
                        }
                    }
                } else {
                    mix_buffer[..current_chunk].to_vec()
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
                        min_rt_factor = min_rt_factor.min(rt_factor);
                        max_rt_factor = max_rt_factor.max(rt_factor);
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

                let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

                let length_in_seconds = current_chunk as f64
                    / audio_info.sample_rate as f64
                    / audio_info.channels as f64;

                #[cfg(feature = "debug")]
                {
                    if started && !buffer_snapshot.is_empty() {
                        let total_samples = current_chunk as f64;
                        let capacity_samples = buffer_snapshot
                            .iter()
                            .map(|(_, buffer)| buffer.lock().unwrap().max_len())
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
                        (avg_chain_time_ms * (1.0 - alpha_chain)) + (chain_time_ms * alpha_chain)
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
                did_work = true;
            }

            if !all_buffers_full && effects_len == 0 {
                #[cfg(feature = "debug")]
                if !did_work {
                    wake_idle = wake_idle.saturating_add(1);
                    let mut metrics = reverb_metrics.lock().unwrap();
                    metrics.wake_total = wake_total;
                    metrics.wake_idle = wake_idle;
                }
                let (guard, _) = buffer_notify
                    .wait_timeout(hash_buffer_copy.lock().unwrap(), Duration::from_millis(20))
                    .unwrap();
                drop(guard);
            } else {
                if did_work {
                    buffer_notify.notify_all();
                }
                drop(buffer_snapshot);
            }
        }
    });

    receiver
}
