//! Core mix-thread runtime loop implementation.

use rodio::buffer::SamplesBuffer;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use crate::audio::buffer::TrackBuffer;
use crate::container::prot::{ShuffleScheduleEntry, ShuffleSource};
use crate::dsp::effects::{convolution_reverb, AudioEffect, EffectContext};
use crate::track::{buffer_container_tracks, ContainerTrackArgs};

use super::super::premix::PremixBuffer;
use super::super::{compute_track_channel_gains, InlineTrackMixUpdate};
use super::effects::run_effect_chain;
use super::source_spawner::SourceSpawner;
use super::track_mix::{mix_tracks_into_premix, TrackMixArgs};
use super::types::ActiveInlineTransition;
use super::MixThreadArgs;

/// Spawn the mixing thread and return a receiver of mixed audio buffers.
pub fn spawn_mix_thread(args: MixThreadArgs) -> mpsc::Receiver<(SamplesBuffer, f64)> {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer, f64)>(1);

    let MixThreadArgs {
        audio_info,
        buffer_map,
        buffer_notify,
        effects_buffer,
        track_weights,
        track_channel_gains,
        effects_reset,
        inline_effects_update,
        inline_track_mix_updates,
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
        const SHUFFLE_CROSSFADE_MS: f64 = 5.0;
        #[cfg(feature = "debug")]
        let mut avg_overrun_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_chain_ksps = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut min_chain_ksps = f64::INFINITY;
        #[cfg(feature = "debug")]
        let mut max_chain_ksps = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut max_overrun_ms = 0.0_f64;
        #[allow(unused_mut)]
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

        let (container_path, mut shuffle_plan) = {
            let prot = prot_locked.lock().unwrap();
            (
                prot.get_container_path(),
                prot.build_runtime_shuffle_plan(start_time),
            )
        };
        let mut active_sources = shuffle_plan.current_sources.clone();
        let upcoming_events: Vec<ShuffleScheduleEntry> =
            shuffle_plan.upcoming_events.drain(..).collect();
        let mut next_shuffle_event_index = 0usize;
        let mut active_track_keys: Vec<u16> = (0..active_sources.len()).map(|i| i as u16).collect();
        let mut next_track_key: u16 = active_track_keys.len() as u16;
        let crossfade_frames = ((audio_info.sample_rate as f64 * SHUFFLE_CROSSFADE_MS) / 1000.0)
            .round()
            .max(1.0) as u32;
        let mut fading_tracks: HashMap<u16, (u32, u32)> = HashMap::new();

        #[cfg(not(feature = "debug"))]
        let _ = &dsp_metrics;

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
        let mut active_inline_transition: Option<ActiveInlineTransition> = None;

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
            let batch_samples =
                convolution_reverb::preferred_batch_samples(audio_info.channels.max(1) as usize);
            if batch_samples > 0 {
                min_mix_samples =
                    ((min_mix_samples + batch_samples - 1) / batch_samples) * batch_samples;
            }
        }
        let mut started = start_samples == 0;
        let mut mix_buffer = vec![0.0_f32; min_mix_samples];
        let mut premix_buffer = PremixBuffer::new();
        let premix_max_samples = (start_samples.max(min_mix_samples).max(1)).saturating_mul(4);
        let warmup_samples = min_mix_samples;
        let track_buffer_size = (audio_info.sample_rate as usize * 10).max(start_samples * 2);
        let slot_channel_gains: Arc<Mutex<Vec<Vec<f32>>>> = Arc::new(Mutex::new({
            let gains = track_channel_gains.lock().unwrap();
            active_track_keys
                .iter()
                .map(|key| {
                    gains
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| vec![1.0; audio_info.channels.max(1) as usize])
                })
                .collect()
        }));
        // Track timeline based on source consumption, not post-DSP output, so
        // shuffle boundaries remain stable even when effects queue tail samples.
        let mut source_timeline_frames =
            (start_time.max(0.0) * audio_info.sample_rate.max(1) as f64).round() as u64;

        let source_spawner = SourceSpawner {
            buffer_map: hash_buffer_copy.clone(),
            buffer_notify: buffer_notify.clone(),
            track_weights: track_weights.clone(),
            track_channel_gains: track_channel_gains.clone(),
            finished_tracks: finished_tracks.clone(),
            abort: abort.clone(),
            container_path: container_path.clone(),
            track_buffer_size,
            output_channels: audio_info.channels as u8,
            fallback_channel_gains: slot_channel_gains.clone(),
        };
        let mut decode_workers: Vec<JoinHandle<()>> = Vec::new();

        let use_container_buffering = container_path.is_some()
            && upcoming_events.is_empty()
            && active_sources
                .iter()
                .all(|source| matches!(source, ShuffleSource::TrackId(_)));
        if use_container_buffering {
            let track_entries: Vec<(u16, u32)> = active_sources
                .iter()
                .enumerate()
                .filter_map(|(slot_index, source)| match source {
                    ShuffleSource::TrackId(track_id) => active_track_keys
                        .get(slot_index)
                        .copied()
                        .map(|key| (key, *track_id)),
                    ShuffleSource::FilePath(_) => None,
                })
                .collect();
            if !track_entries.is_empty() {
                let handle = buffer_container_tracks(
                    ContainerTrackArgs {
                        file_path: container_path.clone().unwrap_or_default(),
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
                decode_workers.push(handle);
            }
        } else {
            for (slot_index, source) in active_sources.iter().enumerate() {
                if let Some(track_key) = active_track_keys.get(slot_index).copied() {
                    if let Some(handle) =
                        source_spawner.spawn(slot_index, track_key, source, start_time)
                    {
                        decode_workers.push(handle);
                    }
                }
            }
        }

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

            let current_source_ms = if audio_info.sample_rate > 0 {
                (source_timeline_frames.saturating_mul(1000)) / audio_info.sample_rate as u64
            } else {
                0
            };
            while next_shuffle_event_index < upcoming_events.len()
                && upcoming_events[next_shuffle_event_index].at_ms <= current_source_ms
            {
                let event = &upcoming_events[next_shuffle_event_index];
                let event_seconds = event.at_ms as f64 / 1000.0;
                for slot_index in 0..event.sources.len() {
                    if slot_index >= active_sources.len() {
                        continue;
                    }
                    if event.sources[slot_index] == active_sources[slot_index] {
                        continue;
                    }
                    if let Some(old_key) = active_track_keys.get(slot_index).copied() {
                        fading_tracks.insert(old_key, (crossfade_frames, crossfade_frames));
                    }
                    let new_key = next_track_key;
                    next_track_key = next_track_key.saturating_add(1);
                    active_sources[slot_index] = event.sources[slot_index].clone();
                    active_track_keys[slot_index] = new_key;
                    if let Some(handle) = source_spawner.spawn(
                        slot_index,
                        new_key,
                        &active_sources[slot_index],
                        event_seconds,
                    ) {
                        decode_workers.push(handle);
                    }
                }
                next_shuffle_event_index += 1;
            }
            apply_pending_track_mix_updates(
                &inline_track_mix_updates,
                &slot_channel_gains,
                &active_track_keys,
                &track_channel_gains,
                audio_info.channels.max(1) as usize,
            );
            let active_key_set: HashSet<u16> = active_track_keys.iter().copied().collect();
            let fading_key_set: HashSet<u16> = fading_tracks.keys().copied().collect();
            let retained_key_set: HashSet<u16> = active_key_set
                .iter()
                .copied()
                .chain(fading_key_set.iter().copied())
                .collect();

            let buffer_snapshot: Vec<(u16, TrackBuffer)> = {
                let map = hash_buffer_copy.lock().unwrap();
                map.iter().map(|(k, v)| (*k, v.clone())).collect()
            };
            let active_buffer_snapshot: Vec<(u16, TrackBuffer)> = buffer_snapshot
                .iter()
                .filter(|(track_key, _)| active_key_set.contains(track_key))
                .map(|(track_key, buffer)| (*track_key, buffer.clone()))
                .collect();
            let fading_buffer_snapshot: Vec<(u16, TrackBuffer)> = buffer_snapshot
                .iter()
                .filter(|(track_key, _)| fading_key_set.contains(track_key))
                .map(|(track_key, buffer)| (*track_key, buffer.clone()))
                .collect();
            let weights_snapshot: HashMap<u16, f32> = {
                let weights = track_weights.lock().unwrap();
                weights.clone()
            };
            let channel_gains_snapshot: HashMap<u16, Vec<f32>> = {
                let gains = track_channel_gains.lock().unwrap();
                gains.clone()
            };
            let mut removable_tracks: Vec<u16> = Vec::new();

            let finished_snapshot = finished_tracks.lock().unwrap().clone();
            let mut all_buffers_full = true;
            for (track_key, buffer) in buffer_snapshot.iter() {
                let len = buffer.lock().unwrap().len();
                if len == 0 {
                    if finished_snapshot.contains(track_key)
                        && !retained_key_set.contains(track_key)
                    {
                        removable_tracks.push(*track_key);
                        continue;
                    }
                    if active_key_set.contains(track_key) {
                        all_buffers_full = false;
                    }
                }
            }

            if !removable_tracks.is_empty() {
                let mut map = hash_buffer_copy.lock().unwrap();
                for track_id in removable_tracks.drain(..) {
                    map.remove(&track_id);
                    fading_tracks.remove(&track_id);
                }
            }

            if active_buffer_snapshot.is_empty()
                && effects_buffer.lock().unwrap().len() == 0
                && premix_buffer.is_empty()
                && next_shuffle_event_index >= upcoming_events.len()
            {
                break;
            }

            if !started {
                let finished = finished_tracks.lock().unwrap();
                let ready = active_buffer_snapshot.iter().all(|(track_key, buffer)| {
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
                premix_buffer.clear();
                active_inline_transition = None;
                inline_effects_update.lock().unwrap().take();
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

            if let Some(update) = inline_effects_update.lock().unwrap().take() {
                let transition_samples = ((update.transition_ms / 1000.0)
                    * audio_info.sample_rate.max(1) as f32)
                    .round() as usize
                    * audio_info.channels.max(1) as usize;
                if transition_samples == 0 {
                    let mut effects_guard = effects.lock().unwrap();
                    *effects_guard = update.effects;
                    for effect in effects_guard.iter_mut() {
                        effect.warm_up(&effect_context);
                    }
                    active_inline_transition = None;
                } else {
                    let old_effects = {
                        let effects_guard = effects.lock().unwrap();
                        effects_guard.clone()
                    };
                    let mut new_effects = update.effects;
                    for effect in new_effects.iter_mut() {
                        effect.warm_up(&effect_context);
                    }
                    active_inline_transition = Some(ActiveInlineTransition {
                        old_effects,
                        new_effects,
                        total_samples: transition_samples,
                        remaining_samples: transition_samples,
                    });
                }
            }

            let effects_len = effects_buffer.lock().unwrap().len();
            let all_tracks_finished = active_track_keys
                .iter()
                .all(|track_key| finished_snapshot.contains(track_key))
                && next_shuffle_event_index >= upcoming_events.len();
            let active_min_len = active_buffer_snapshot
                .iter()
                .filter(|(track_key, _)| !finished_snapshot.contains(track_key))
                .map(|(_, buffer)| buffer.lock().unwrap().len())
                .min()
                .unwrap_or(0);
            let finished_min_len = active_buffer_snapshot
                .iter()
                .filter(|(track_key, _)| finished_snapshot.contains(track_key))
                .map(|(_, buffer)| buffer.lock().unwrap().len())
                .min()
                .unwrap_or(0);
            let has_tail = effects_len > 0;
            let should_mix_tracks = !active_buffer_snapshot.is_empty()
                && premix_buffer.len() < premix_max_samples
                && ((!all_tracks_finished && active_min_len >= min_mix_samples)
                    || (all_tracks_finished && finished_min_len > 0));

            #[cfg(feature = "debug")]
            if started
                && startup_log_start.elapsed().as_secs_f64() <= 8.0
                && last_startup_log.elapsed().as_millis() >= 200
            {
                let mut sizes: Vec<String> = Vec::new();
                for (track_key, buffer) in active_buffer_snapshot.iter() {
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
            if started && !should_mix_tracks && !has_tail && premix_buffer.is_empty() {
                underrun_count = underrun_count.saturating_add(1);
                if last_underrun_log.elapsed().as_secs_f64() >= 1.0 {
                    log::warn!(
                        "DSP underrun: active_min_len={} finished_min_len={} effects_len={} tracks={} finished={}",
                        active_min_len,
                        finished_min_len,
                        effects_len,
                        active_buffer_snapshot.len(),
                        finished_snapshot.len()
                    );
                    last_underrun_log = Instant::now();
                }
                if let Ok(mut metrics) = dsp_metrics.lock() {
                    metrics.underrun_count = underrun_count;
                    metrics.underrun_active = true;
                }
            }

            if active_buffer_snapshot.is_empty()
                && effects_len == 0
                && premix_buffer.is_empty()
                && all_tracks_finished
            {
                let drained = if let Some(transition) = active_inline_transition.as_mut() {
                    let old_out =
                        run_effect_chain(&mut transition.old_effects, &[], &effect_context, true);
                    let new_out =
                        run_effect_chain(&mut transition.new_effects, &[], &effect_context, true);
                    let len = old_out.len().max(new_out.len());
                    let mut blended = Vec::with_capacity(len);
                    for sample_index in 0..len {
                        let old_sample = old_out.get(sample_index).copied().unwrap_or(0.0);
                        let new_sample = new_out.get(sample_index).copied().unwrap_or(0.0);
                        let mix = if transition.total_samples == 0 {
                            1.0
                        } else {
                            let completed = transition
                                .total_samples
                                .saturating_sub(transition.remaining_samples);
                            completed as f32 / transition.total_samples as f32
                        }
                        .clamp(0.0, 1.0);
                        blended.push((old_sample * (1.0 - mix)) + (new_sample * mix));
                    }
                    blended
                } else {
                    let mut effects_guard = effects.lock().unwrap();
                    run_effect_chain(&mut effects_guard, &[], &effect_context, true)
                };
                if !drained.is_empty() {
                    let mut tail_buffer = effects_buffer.lock().unwrap();
                    for sample in drained {
                        let _ = tail_buffer.push(sample);
                    }
                    continue;
                }
                break;
            }

            if should_mix_tracks || has_tail || !premix_buffer.is_empty() {
                let input_channels = audio_info.channels as u16;
                let sample_rate = audio_info.sample_rate as u32;
                let channel_count = input_channels.max(1) as usize;

                if should_mix_tracks {
                    let next_event_ms = upcoming_events
                        .get(next_shuffle_event_index)
                        .map(|event| event.at_ms);
                    let (consumed_source_frames, mixed) = mix_tracks_into_premix(TrackMixArgs {
                        mix_buffer: &mut mix_buffer,
                        premix_buffer: &mut premix_buffer,
                        active_buffer_snapshot: &active_buffer_snapshot,
                        fading_buffer_snapshot: &fading_buffer_snapshot,
                        weights_snapshot: &weights_snapshot,
                        channel_gains_snapshot: &channel_gains_snapshot,
                        fading_tracks: &mut fading_tracks,
                        min_mix_samples,
                        premix_max_samples,
                        all_tracks_finished,
                        active_min_len,
                        finished_min_len,
                        next_event_ms,
                        current_source_ms,
                        sample_rate,
                        channel_count,
                    });
                    source_timeline_frames =
                        source_timeline_frames.saturating_add(consumed_source_frames);
                    if mixed {
                        did_work = true;
                    }
                }

                let samples = super::output_stage::produce_output_samples(
                    super::output_stage::OutputStageArgs {
                        effects_buffer: &effects_buffer,
                        premix_buffer: &mut premix_buffer,
                        min_mix_samples,
                        all_tracks_finished,
                        input_channels,
                        sample_rate,
                        effect_context: &effect_context,
                        active_inline_transition: &mut active_inline_transition,
                        effects: &effects,
                        buffer_settings: &buffer_settings,
                        dsp_metrics: &dsp_metrics,
                        underrun_count,
                        track_key_count: active_buffer_snapshot.len(),
                        finished_track_count: finished_snapshot.len(),
                        prot_key_count: active_track_keys.len(),
                        #[cfg(feature = "debug")]
                        avg_overrun_ms: &mut avg_overrun_ms,
                        #[cfg(feature = "debug")]
                        avg_chain_ksps: &mut avg_chain_ksps,
                        #[cfg(feature = "debug")]
                        min_chain_ksps: &mut min_chain_ksps,
                        #[cfg(feature = "debug")]
                        max_chain_ksps: &mut max_chain_ksps,
                        #[cfg(feature = "debug")]
                        max_overrun_ms: &mut max_overrun_ms,
                        #[cfg(feature = "debug")]
                        pop_count: &mut pop_count,
                        #[cfg(feature = "debug")]
                        clip_count: &mut clip_count,
                        #[cfg(feature = "debug")]
                        nan_count: &mut nan_count,
                        #[cfg(feature = "debug")]
                        last_pop_log: &mut last_pop_log,
                        #[cfg(feature = "debug")]
                        last_boundary_log: &mut last_boundary_log,
                        #[cfg(feature = "debug")]
                        last_samples: &mut last_samples,
                        #[cfg(feature = "debug")]
                        boundary_initialized: &mut boundary_initialized,
                        #[cfg(feature = "debug")]
                        boundary_count: &mut boundary_count,
                        #[cfg(feature = "debug")]
                        effect_boundary_initialized: &mut effect_boundary_initialized,
                        #[cfg(feature = "debug")]
                        effect_last_samples: &mut effect_last_samples,
                        #[cfg(feature = "debug")]
                        effect_boundary_counts: &mut effect_boundary_counts,
                        #[cfg(feature = "debug")]
                        effect_boundary_logs: &mut effect_boundary_logs,
                        #[cfg(feature = "debug")]
                        alpha,
                    },
                );

                if super::output_stage::send_samples(&sender, input_channels, sample_rate, samples)
                {
                    did_work = true;
                }
            }

            if !all_buffers_full
                && effects_buffer.lock().unwrap().len() == 0
                && premix_buffer.is_empty()
            {
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

        for worker in decode_workers {
            if worker.join().is_err() {
                log::warn!("decode worker panicked during join");
            }
        }
    });

    receiver
}

fn apply_pending_track_mix_updates(
    inline_track_mix_updates: &Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    slot_channel_gains: &Arc<Mutex<Vec<Vec<f32>>>>,
    active_track_keys: &[u16],
    track_channel_gains: &Arc<Mutex<HashMap<u16, Vec<f32>>>>,
    channels: usize,
) {
    let updates = {
        let mut pending = inline_track_mix_updates.lock().unwrap();
        std::mem::take(&mut *pending)
    };
    if updates.is_empty() {
        return;
    }

    let mut slot_gains = slot_channel_gains.lock().unwrap();
    let mut key_gains = track_channel_gains.lock().unwrap();
    for update in updates {
        if update.slot_index >= slot_gains.len() {
            continue;
        }
        let gains = compute_track_channel_gains(update.level, update.pan, channels);
        slot_gains[update.slot_index] = gains.clone();
        if let Some(track_key) = active_track_keys.get(update.slot_index).copied() {
            key_gains.insert(track_key, gains);
        }
    }
}
