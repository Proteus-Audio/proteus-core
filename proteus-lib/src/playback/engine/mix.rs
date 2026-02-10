//! Mixing thread for combining buffered tracks and applying effects.

use dasp_ring_buffer::Bounded;
use log::error;
use rodio::buffer::SamplesBuffer;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::audio::buffer::TrackBuffer;
use crate::container::prot::Prot;
use crate::dsp::effects::{convolution_reverb, AudioEffect, EffectContext};
use crate::track::{buffer_container_tracks, buffer_track, ContainerTrackArgs, TrackArgs};

use super::state::{DspChainMetrics, PlaybackBufferSettings};

#[cfg(feature = "debug")]
fn effect_label(effect: &AudioEffect) -> &'static str {
    match effect {
        AudioEffect::DelayReverb(_) => "DelayReverb",
        AudioEffect::BasicReverb(_) => "BasicReverb",
        AudioEffect::DiffusionReverb(_) => "DiffusionReverb",
        AudioEffect::ConvolutionReverb(_) => "ConvolutionReverb",
        AudioEffect::LowPassFilter(_) => "LowPassFilter",
        AudioEffect::HighPassFilter(_) => "HighPassFilter",
        AudioEffect::Distortion(_) => "Distortion",
        AudioEffect::Compressor(_) => "Compressor",
        AudioEffect::Limiter(_) => "Limiter",
    }
}

/// Arguments required to spawn the mixing thread.
pub struct MixThreadArgs {
    pub audio_info: crate::container::info::Info,
    pub buffer_map: Arc<Mutex<HashMap<u16, TrackBuffer>>>,
    pub buffer_notify: Arc<std::sync::Condvar>,
    pub effects_buffer: Arc<Mutex<Bounded<Vec<f32>>>>,
    pub track_weights: Arc<Mutex<HashMap<u16, f32>>>,
    pub effects_reset: Arc<AtomicU64>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub prot: Arc<Mutex<Prot>>,
    pub abort: Arc<AtomicBool>,
    pub start_time: f64,
    pub buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub dsp_metrics: Arc<Mutex<DspChainMetrics>>,
}

/// Spawn the mixing thread and return a receiver of mixed audio buffers.
pub fn spawn_mix_thread(args: MixThreadArgs) -> mpsc::Receiver<(SamplesBuffer, f64)> {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer, f64)>(1);

    let MixThreadArgs {
        audio_info,
        buffer_map,
        buffer_notify,
        effects_buffer,
        track_weights,
        effects_reset,
        finished_tracks,
        prot,
        abort,
        start_time,
        buffer_settings,
        effects,
        dsp_metrics,
    } = args;

    thread::spawn(move || {
        const MIN_MIX_MS: f32 = 300.0;
        #[cfg(feature = "debug")]
        let mut avg_dsp_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_audio_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_rt_factor = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_overrun_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut min_rt_factor = f64::INFINITY;
        #[cfg(feature = "debug")]
        let mut max_rt_factor = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_chain_ksps = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut min_chain_ksps = f64::INFINITY;
        #[cfg(feature = "debug")]
        let mut max_chain_ksps = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut max_overrun_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut underrun_count = 0_u64;
        #[cfg(feature = "debug")]
        let mut last_underrun_log = Instant::now();
        #[cfg(feature = "debug")]
        let mut last_startup_log = Instant::now();
        #[cfg(feature = "debug")]
        let startup_log_start = Instant::now();
        #[cfg(feature = "debug")]
        let mut last_pop_log = Instant::now();
        #[cfg(feature = "debug")]
        let mut last_boundary_log = Instant::now();
        #[cfg(feature = "debug")]
        let mut pop_count = 0_u64;
        #[cfg(feature = "debug")]
        let mut clip_count = 0_u64;
        #[cfg(feature = "debug")]
        let mut nan_count = 0_u64;
        #[cfg(feature = "debug")]
        let mut last_samples: Vec<f32> = Vec::new();
        #[cfg(feature = "debug")]
        let mut boundary_initialized = false;
        #[cfg(feature = "debug")]
        let mut boundary_count = 0_u64;
        #[cfg(feature = "debug")]
        let mut effect_boundary_initialized: Vec<bool> = Vec::new();
        #[cfg(feature = "debug")]
        let mut effect_last_samples: Vec<Vec<f32>> = Vec::new();
        #[cfg(feature = "debug")]
        let mut effect_boundary_counts: Vec<u64> = Vec::new();
        #[cfg(feature = "debug")]
        let mut effect_boundary_logs: Vec<Instant> = Vec::new();
        #[cfg(feature = "debug")]
        let alpha = 0.1_f64;

        let prot_locked = prot.clone();

        let prot = prot_locked.lock().unwrap();
        let container_tracks = prot.container_track_entries();
        let enumerated_list = if container_tracks.is_some() {
            Vec::new()
        } else {
            prot.enumerated_list()
        };
        let prot_key_count = prot.get_keys().len();
        drop(prot);

        #[cfg(not(feature = "debug"))]
        let _ = &dsp_metrics;

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
                    track_eos_ms: buffer_settings.lock().unwrap().track_eos_ms,
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

        let mut effect_context = {
            let prot = prot_locked.lock().unwrap();
            EffectContext {
                sample_rate: prot.info.sample_rate,
                channels: prot.info.channels as usize,
                container_path: prot.get_container_path(),
                impulse_response_spec: prot.get_impulse_response_spec(),
                impulse_response_tail_db: prot.get_impulse_response_tail_db().unwrap_or(-60.0),
            }
        };

        let mut last_effects_reset = effects_reset.load(std::sync::atomic::Ordering::SeqCst);

        let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
        let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0) as usize
            * audio_info.channels as usize;
        let min_mix_frames = ((audio_info.sample_rate as f32 * MIN_MIX_MS) / 1000.0) as usize;
        let mut min_mix_samples = min_mix_frames.max(1) * audio_info.channels as usize;
        let has_convolution = {
            let effects_guard = effects.lock().unwrap();
            effects_guard.iter().any(|effect| match effect {
                AudioEffect::ConvolutionReverb(effect) => effect.enabled,
                _ => false,
            })
        };
        if has_convolution {
            let batch_samples = convolution_reverb::preferred_batch_samples(
                audio_info.channels.max(1) as usize,
            );
            if batch_samples > 0 {
                min_mix_samples =
                    ((min_mix_samples + batch_samples - 1) / batch_samples) * batch_samples;
            }
        }
        let mut started = start_samples == 0;
        let mut mix_buffer = vec![0.0_f32; min_mix_samples];
        let warmup_samples = min_mix_samples;

        if warmup_samples > 0 {
            let warmup_start = Instant::now();
            let mut processed = vec![0.0_f32; warmup_samples];
            {
                let mut effects_guard = effects.lock().unwrap();
                for effect in effects_guard.iter_mut() {
                    processed = effect.process(&processed, &effect_context, false);
                }
            }
            {
                let mut tail_buffer = effects_buffer.lock().unwrap();
                while tail_buffer.pop().is_some() {}
            }
            log::info!(
                "DSP warmup: {:.2}ms ({} samples)",
                warmup_start.elapsed().as_secs_f64() * 1000.0,
                warmup_samples
            );
        }

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

            let finished_snapshot = finished_tracks.lock().unwrap().clone();
            let mut all_buffers_full = true;
            for (track_key, buffer) in buffer_snapshot.iter() {
                let len = buffer.lock().unwrap().len();
                if len == 0 {
                    if finished_snapshot.contains(track_key) {
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
                    let (guard, _) = buffer_notify
                        .wait_timeout(hash_buffer_copy.lock().unwrap(), Duration::from_millis(20))
                        .unwrap();
                    drop(guard);
                    continue;
                }
            }

            let mut did_work = false;
            let current_reset = effects_reset.load(std::sync::atomic::Ordering::SeqCst);
            if current_reset != last_effects_reset {
                let mut effects_guard = effects.lock().unwrap();
                for effect in effects_guard.iter_mut() {
                    effect.reset_state();
                }
                let mut tail_buffer = effects_buffer.lock().unwrap();
                while tail_buffer.pop().is_some() {}
                effect_context = {
                    let prot = prot_locked.lock().unwrap();
                    EffectContext {
                        sample_rate: prot.info.sample_rate,
                        channels: prot.info.channels as usize,
                        container_path: prot.get_container_path(),
                        impulse_response_spec: prot.get_impulse_response_spec(),
                        impulse_response_tail_db: prot
                            .get_impulse_response_tail_db()
                            .unwrap_or(-60.0),
                    }
                };
                last_effects_reset = current_reset;
                #[cfg(feature = "debug")]
                {
                    effect_boundary_initialized.clear();
                    effect_last_samples.clear();
                    effect_boundary_counts.clear();
                    effect_boundary_logs.clear();
                }
            }

            let effects_len = effects_buffer.lock().unwrap().len();
            let all_tracks_finished = finished_snapshot.len() >= prot_key_count;
            let active_min_len = buffer_snapshot
                .iter()
                .filter(|(track_key, _)| !finished_snapshot.contains(track_key))
                .map(|(_, buffer)| buffer.lock().unwrap().len())
                .min()
                .unwrap_or(0);
            let finished_min_len = buffer_snapshot
                .iter()
                .filter(|(track_key, _)| finished_snapshot.contains(track_key))
                .map(|(_, buffer)| buffer.lock().unwrap().len())
                .min()
                .unwrap_or(0);
            let has_tail = effects_len > 0;
            let should_output_tail = has_tail;
            let should_mix_tracks = !has_tail
                && !buffer_snapshot.is_empty()
                && ((!all_tracks_finished && active_min_len >= min_mix_samples)
                    || (all_tracks_finished && finished_min_len > 0));

            #[cfg(feature = "debug")]
            if started
                && startup_log_start.elapsed().as_secs_f64() <= 8.0
                && last_startup_log.elapsed().as_millis() >= 200
            {
                let mut sizes: Vec<String> = Vec::new();
                for (track_key, buffer) in buffer_snapshot.iter() {
                    let len = buffer.lock().unwrap().len();
                    sizes.push(format!("{}={}", track_key, len));
                }
                log::debug!(
                    "startup buffers: t={:.2}s active_min={} finished_min={} tail={} sizes=[{}]",
                    startup_log_start.elapsed().as_secs_f64(),
                    active_min_len,
                    finished_min_len,
                    effects_len,
                    sizes.join(", ")
                );
                last_startup_log = Instant::now();
            }

            #[cfg(feature = "debug")]
            if started && !should_mix_tracks && !should_output_tail {
                underrun_count = underrun_count.saturating_add(1);
                if last_underrun_log.elapsed().as_secs_f64() >= 1.0 {
                    log::warn!(
                        "DSP underrun: active_min_len={} finished_min_len={} effects_len={} tracks={} finished={}",
                        active_min_len,
                        finished_min_len,
                        effects_len,
                        buffer_snapshot.len(),
                        finished_snapshot.len()
                    );
                    last_underrun_log = Instant::now();
                }
                if let Ok(mut metrics) = dsp_metrics.lock() {
                    metrics.underrun_count = underrun_count;
                    metrics.underrun_active = true;
                }
            }

            if buffer_snapshot.is_empty() && effects_len == 0 && all_tracks_finished {
                let mut drained = Vec::new();
                {
                    let mut effects_guard = effects.lock().unwrap();
                    let mut current = Vec::new();
                    for effect in effects_guard.iter_mut() {
                        current = effect.process(&current, &effect_context, true);
                    }
                    drained = current;
                }
                if !drained.is_empty() {
                    let mut tail_buffer = effects_buffer.lock().unwrap();
                    for sample in drained {
                        let _ = tail_buffer.push(sample);
                    }
                    continue;
                }
                break;
            }

            if should_mix_tracks || should_output_tail {
                let input_channels = audio_info.channels as u16;
                let sample_rate = audio_info.sample_rate as u32;

                let mut samples = if should_mix_tracks {
                    let current_chunk = if !all_tracks_finished && active_min_len >= min_mix_samples
                    {
                        min_mix_samples
                    } else if all_tracks_finished && finished_min_len > 0 {
                        finished_min_len
                    } else {
                        0
                    };
                    let current_chunk = current_chunk.min(mix_buffer.len());

                    if current_chunk == 0 {
                        Vec::new()
                    } else {
                        mix_buffer.fill(0.0);
                        for (track_key, buffer) in buffer_snapshot.iter() {
                            let weight = weights_snapshot.get(track_key).copied().unwrap_or(1.0);
                            let mut buffer = buffer.lock().unwrap();
                            let take = buffer.len().min(current_chunk);
                            for sample in mix_buffer.iter_mut().take(take) {
                                if let Some(value) = buffer.pop() {
                                    *sample += value * weight;
                                }
                            }
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

                        let drain_effects = all_tracks_finished;
                        let mut processed = mix_buffer[..current_chunk].to_vec();
                        {
                            let mut effects_guard = effects.lock().unwrap();
                            #[cfg(feature = "debug")]
                            let effect_boundary_log = {
                                let settings = buffer_settings.lock().unwrap();
                                settings.effect_boundary_log
                            };
                            #[cfg(feature = "debug")]
                            if effect_boundary_log {
                                let effect_count = effects_guard.len();
                                if effect_boundary_initialized.len() != effect_count
                                    || effect_last_samples.len() != effect_count
                                    || effect_boundary_counts.len() != effect_count
                                    || effect_boundary_logs.len() != effect_count
                                {
                                    let channels = input_channels.max(1) as usize;
                                    effect_boundary_initialized = vec![false; effect_count];
                                    effect_last_samples = vec![vec![0.0; channels]; effect_count];
                                    effect_boundary_counts = vec![0_u64; effect_count];
                                    effect_boundary_logs = vec![Instant::now(); effect_count];
                                }
                            }
                            for (effect_index, effect) in effects_guard.iter_mut().enumerate() {
                                #[cfg(not(feature = "debug"))]
                                let _ = effect_index;
                                processed =
                                    effect.process(&processed, &effect_context, drain_effects);
                                #[cfg(feature = "debug")]
                                if effect_boundary_log {
                                    let channels = input_channels.max(1) as usize;
                                    if effect_index < effect_last_samples.len()
                                        && effect_index < effect_boundary_initialized.len()
                                    {
                                        if processed.len() >= channels {
                                            let initialized =
                                                effect_boundary_initialized[effect_index];
                                            for ch in 0..channels {
                                                let prev = effect_last_samples[effect_index][ch];
                                                let curr = processed[ch];
                                                let delta = (curr - prev).abs();
                                                if initialized && delta > 0.1 {
                                                    effect_boundary_counts[effect_index] =
                                                        effect_boundary_counts[effect_index]
                                                            .saturating_add(1);
                                                    if effect_boundary_logs[effect_index]
                                                        .elapsed()
                                                        .as_millis()
                                                        >= 200
                                                    {
                                                        log::info!(
                                                            "effect boundary discontinuity: effect={} delta={:.4} prev={:.4} curr={:.4} ch={} count={}",
                                                            effect_label(effect),
                                                            delta,
                                                            prev,
                                                            curr,
                                                            ch,
                                                            effect_boundary_counts[effect_index]
                                                        );
                                                        effect_boundary_logs[effect_index] =
                                                            Instant::now();
                                                    }
                                                }
                                            }
                                            let last_frame_start =
                                                processed.len().saturating_sub(channels);
                                            for ch in 0..channels {
                                                let idx = (last_frame_start + ch)
                                                    .min(processed.len().saturating_sub(1));
                                                effect_last_samples[effect_index][ch] =
                                                    processed[idx];
                                            }
                                            if !effect_boundary_initialized[effect_index]
                                                && !processed.is_empty()
                                            {
                                                effect_boundary_initialized[effect_index] = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if processed.len() < current_chunk {
                            processed.resize(current_chunk, 0.0);
                        } else if processed.len() > current_chunk {
                            let extra = processed.split_off(current_chunk);
                            let mut tail_buffer = effects_buffer.lock().unwrap();
                            for sample in extra {
                                let _ = tail_buffer.push(sample);
                            }
                        }

                        #[cfg(feature = "debug")]
                        {
                            let channels = input_channels.max(1) as usize;
                            if last_samples.len() != channels {
                                last_samples = vec![0.0; channels];
                            }
                            for (idx, sample) in processed.iter().enumerate() {
                                let ch = idx % channels;
                                let prev = last_samples[ch];
                                if sample.is_nan() || sample.is_infinite() {
                                    nan_count = nan_count.saturating_add(1);
                                }
                                if sample.abs() > 1.0 {
                                    clip_count = clip_count.saturating_add(1);
                                }
                                let delta = (sample - prev).abs();
                                if boundary_initialized && idx < channels && delta > 0.1 {
                                    boundary_count = boundary_count.saturating_add(1);
                                    if last_boundary_log.elapsed().as_millis() >= 200 {
                                        log::info!(
                                            "boundary discontinuity: delta={:.4} prev={:.4} curr={:.4} ch={} count={}",
                                            delta,
                                            prev,
                                            sample,
                                            ch,
                                            boundary_count
                                        );
                                        last_boundary_log = Instant::now();
                                    }
                                }
                                if delta > 0.9 && sample.abs() > 0.6 {
                                    pop_count = pop_count.saturating_add(1);
                                }
                                last_samples[ch] = *sample;
                            }
                            if !boundary_initialized && !processed.is_empty() {
                                boundary_initialized = true;
                            }
                            if last_pop_log.elapsed().as_secs_f64() >= 1.0
                                && (pop_count > 0 || clip_count > 0 || nan_count > 0)
                            {
                                log::warn!(
                                    "sample anomalies: pops={} clips={} nans={}",
                                    pop_count,
                                    clip_count,
                                    nan_count
                                );
                                last_pop_log = Instant::now();
                            }
                        }

                        #[cfg(feature = "debug")]
                        {
                            let dsp_time_ms = dsp_start.elapsed().as_secs_f64() * 1000.0;
                            let rt_factor = if audio_time_ms > 0.0 {
                                dsp_time_ms / audio_time_ms
                            } else {
                                0.0
                            };
                            let overrun_ms = (dsp_time_ms - audio_time_ms).max(0.0);
                            let overrun = rt_factor > 1.0;
                            let chain_ksps = if dsp_time_ms > 0.0 {
                                (processed.len() as f64 / (dsp_time_ms / 1000.0)) / 1000.0
                            } else {
                                0.0
                            };

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
                            avg_overrun_ms = if avg_overrun_ms == 0.0 {
                                overrun_ms
                            } else {
                                (avg_overrun_ms * (1.0 - alpha)) + (overrun_ms * alpha)
                            };
                            avg_chain_ksps = if avg_chain_ksps == 0.0 {
                                chain_ksps
                            } else {
                                (avg_chain_ksps * (1.0 - alpha)) + (chain_ksps * alpha)
                            };

                            if rt_factor > 0.0 {
                                min_rt_factor = min_rt_factor.min(rt_factor);
                                max_rt_factor = max_rt_factor.max(rt_factor);
                            }
                            if overrun_ms > 0.0 {
                                max_overrun_ms = max_overrun_ms.max(overrun_ms);
                            }
                            if chain_ksps > 0.0 {
                                min_chain_ksps = min_chain_ksps.min(chain_ksps);
                                max_chain_ksps = max_chain_ksps.max(chain_ksps);
                            }

                            let mut metrics = dsp_metrics.lock().unwrap();
                            metrics.dsp_time_ms = dsp_time_ms;
                            metrics.audio_time_ms = audio_time_ms;
                            metrics.rt_factor = rt_factor;
                            metrics.overrun = overrun;
                            metrics.overrun_ms = overrun_ms;
                            metrics.avg_overrun_ms = avg_overrun_ms;
                            metrics.max_overrun_ms = max_overrun_ms;
                            metrics.avg_dsp_ms = avg_dsp_ms;
                            metrics.avg_audio_ms = avg_audio_ms;
                            metrics.avg_rt_factor = avg_rt_factor;
                            metrics.min_rt_factor = if min_rt_factor.is_finite() {
                                min_rt_factor
                            } else {
                                0.0
                            };
                            metrics.max_rt_factor = max_rt_factor;
                            metrics.chain_ksps = chain_ksps;
                            metrics.avg_chain_ksps = avg_chain_ksps;
                            metrics.min_chain_ksps = if min_chain_ksps.is_finite() {
                                min_chain_ksps
                            } else {
                                0.0
                            };
                            metrics.max_chain_ksps = max_chain_ksps;
                            metrics.underrun_count = underrun_count;
                            metrics.underrun_active = false;
                            metrics.pop_count = pop_count;
                            metrics.clip_count = clip_count;
                            metrics.nan_count = nan_count;
                            metrics.track_key_count = buffer_snapshot.len();
                            metrics.finished_track_count = finished_snapshot.len();
                            metrics.prot_key_count = prot_key_count;
                        }

                        processed
                    }
                } else {
                    let mut tail_buffer = effects_buffer.lock().unwrap();
                    let take = tail_buffer.len().min(min_mix_samples).max(1);
                    let mut out = Vec::with_capacity(take);
                    for _ in 0..take {
                        if let Some(sample) = tail_buffer.pop() {
                            out.push(sample);
                        }
                    }
                    out
                };

                if !samples.is_empty() {
                    let length_in_seconds = samples.len() as f64
                        / audio_info.sample_rate as f64
                        / audio_info.channels as f64;
                    let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

                    match sender.send((samples_buffer, length_in_seconds)) {
                        Ok(_) => (),
                        Err(e) => {
                            error!("Failed to send samples: {}", e);
                        }
                    }
                    did_work = true;
                }
            }

            if !all_buffers_full && effects_buffer.lock().unwrap().len() == 0 {
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
