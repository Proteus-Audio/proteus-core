//! Core mix-thread runtime loop implementation.

mod decode;

use rodio::buffer::SamplesBuffer;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use log::{debug, info, warn};

use crate::dsp::effects::{convolution_reverb, AudioEffect, EffectContext};
#[cfg(feature = "debug")]
use crate::logging::pivot_buffer_trace::pivot_buffer;

use super::buffer_mixer::{BufferMixer, SourceKey};
use super::decoder_events::DecodeWorkerEvent;
use super::effects::run_effect_chain;
use super::types::ActiveInlineTransition;
use super::MixThreadArgs;
use decode::{spawn_container_decode_worker, spawn_file_decode_worker, DecodeWorkerJoinGuard};

const MAX_EFFECT_DRAIN_PASSES: usize = 1024;
const DRAIN_SILENCE_EPSILON: f32 = 1.0e-6;
const DRAIN_SILENT_PASSES_TO_STOP: usize = 2;

/// Spawn the mixing thread and return a receiver of mixed audio buffers.
pub fn spawn_mix_thread(
    args: MixThreadArgs,
) -> (mpsc::Receiver<(SamplesBuffer, f64)>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::sync_channel::<(SamplesBuffer, f64)>(1);

    let MixThreadArgs {
        audio_info,
        buffer_map: _buffer_map,
        buffer_notify,
        effects_buffer: _effects_buffer,
        track_weights: _track_weights,
        track_channel_gains: _track_channel_gains,
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

    let handle = thread::spawn(move || {
        let startup_trace = Instant::now();
        info!("mix startup trace: thread start");
        const MIN_MIX_MS: f32 = 30.0;
        let mut running_count = 0;

        let prot_locked = prot.clone();
        let (instance_plan, container_path, mut effect_context, track_mix_settings_by_slot) = {
            let prot = prot_locked.lock().unwrap();
            (
                prot.build_runtime_instance_plan(start_time),
                prot.get_container_path(),
                EffectContext {
                    sample_rate: prot.info.sample_rate,
                    channels: prot.info.channels as usize,
                    container_path: prot.get_container_path(),
                    impulse_response_spec: prot.get_impulse_response_spec(),
                    impulse_response_tail_db: prot.get_impulse_response_tail_db().unwrap_or(-60.0),
                },
                prot.get_track_mix_settings(),
            )
        };
        info!(
            "mix startup trace: runtime plan built in {}ms (instances={})",
            startup_trace.elapsed().as_millis(),
            instance_plan.instances.len()
        );

        if instance_plan.instances.is_empty() {
            abort.store(true, Ordering::SeqCst);
            return;
        }

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
                min_mix_samples = min_mix_samples.div_ceil(batch_samples) * batch_samples;
            }
        }
        let convolution_batch_samples = if has_convolution {
            convolution_reverb::preferred_batch_samples(audio_info.channels.max(1) as usize)
        } else {
            0
        };

        let mut track_mix_by_logical: HashMap<usize, (f32, f32)> = HashMap::new();
        for instance in instance_plan.instances.iter() {
            track_mix_by_logical
                .entry(instance.logical_track_index)
                .or_insert_with(|| {
                    track_mix_settings_by_slot
                        .get(&(instance.slot_index as u16))
                        .copied()
                        .unwrap_or((1.0, 0.0))
                });
        }

        // BufferMixer stores interleaved samples, so capacity must be expressed in samples
        // (not frames). The previous `sample_rate * 10` sizing effectively halved capacity
        // for stereo and increased overflow risk once zero-fill alignment was enabled.
        let track_buffer_size = ((audio_info.sample_rate as usize * 10)
            * audio_info.channels.max(1) as usize)
            .max(start_samples * 2);
        let mut buffer_mixer = BufferMixer::new(
            instance_plan,
            audio_info.sample_rate,
            audio_info.channels.max(1) as usize,
            track_buffer_size,
            track_mix_by_logical,
            min_mix_samples,
        );
        info!(
            "mix startup trace: buffer_mixer ready in {}ms (track_buffer_size={} min_mix_samples={} start_samples={})",
            startup_trace.elapsed().as_millis(),
            track_buffer_size,
            min_mix_samples,
            start_samples
        );
        let decode_backpressure = buffer_mixer.decode_backpressure();

        let (packet_tx, packet_rx) = mpsc::sync_channel::<DecodeWorkerEvent>(64);
        let mut decode_workers = DecodeWorkerJoinGuard::default();

        let mut track_ids = HashSet::new();
        let mut file_paths = HashSet::new();
        for source in buffer_mixer.sources() {
            match source {
                SourceKey::TrackId(track_id) => {
                    track_ids.insert(track_id);
                }
                SourceKey::FilePath(path) => {
                    file_paths.insert(path);
                }
            }
        }
        let startup_gate_target_samples = start_samples.max(min_mix_samples);
        if !file_paths.is_empty() && startup_gate_target_samples > 0 {
            decode_backpressure.enable_startup_priority(startup_gate_target_samples);
        }

        if !track_ids.is_empty() {
            if let Some(path) = container_path {
                decode_workers.push(spawn_container_decode_worker(
                    path,
                    track_ids.into_iter().collect(),
                    start_time,
                    audio_info.channels as u8,
                    packet_tx.clone(),
                    abort.clone(),
                    decode_backpressure.clone(),
                ));
            }
        }

        for path in file_paths {
            decode_workers.push(spawn_file_decode_worker(
                path,
                start_time,
                audio_info.channels as u8,
                packet_tx.clone(),
                abort.clone(),
                decode_backpressure.clone(),
            ));
        }
        drop(packet_tx);
        info!(
            "mix startup trace: decode workers spawned in {}ms (container_sources={} file_sources={})",
            startup_trace.elapsed().as_millis(),
            if buffer_mixer
                .sources()
                .iter()
                .any(|s| matches!(s, SourceKey::TrackId(_)))
            {
                1
            } else {
                0
            },
            buffer_mixer
                .sources()
                .iter()
                .filter(|s| matches!(s, SourceKey::FilePath(_)))
                .count()
        );

        let warmup_samples = min_mix_samples;
        if warmup_samples > 0 {
            let mut effects_guard = effects.lock().unwrap();
            for effect in effects_guard.iter_mut() {
                effect.warm_up(&effect_context);
            }
        }
        info!(
            "mix startup trace: effect warmup complete in {}ms (warmup_samples={})",
            startup_trace.elapsed().as_millis(),
            warmup_samples
        );

        let mut started = start_samples == 0;
        let mut last_effects_reset = effects_reset.load(Ordering::SeqCst);
        let mut active_inline_transition: Option<ActiveInlineTransition> = None;
        let mut pending_mix_samples: Vec<f32> = Vec::new();
        #[cfg(feature = "debug")]
        let alpha = 0.1_f64;
        #[cfg(feature = "debug")]
        let mut avg_overrun_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut max_overrun_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_chain_ksps = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut min_chain_ksps = f64::INFINITY;
        #[cfg(feature = "debug")]
        let mut max_chain_ksps = 0.0_f64;
        let mut logged_first_packet_drain = false;
        let mut logged_first_packet_route = false;
        let mut logged_start_gate = false;
        let mut logged_first_take_samples = false;
        let mut logged_first_output_send = false;
        let mut effect_drain_passes = 0usize;
        let mut effect_drain_silent_passes = 0usize;

        loop {
            if abort.load(Ordering::SeqCst) {
                break;
            }

            drain_decode_events(
                &packet_rx,
                &mut buffer_mixer,
                startup_trace,
                &mut logged_first_packet_drain,
                &mut logged_first_packet_route,
            );
            apply_inline_track_mix_updates(&inline_track_mix_updates, &mut buffer_mixer);
            apply_effect_runtime_updates(
                &effects_reset,
                &mut last_effects_reset,
                &effects,
                &mut active_inline_transition,
                &inline_effects_update,
                &prot_locked,
                &audio_info,
                &mut effect_context,
            );

            if !started {
                if buffer_mixer.mix_ready_with_min_samples(start_samples.max(min_mix_samples)) {
                    started = true;
                    decode_backpressure.disable_startup_priority();
                    if !logged_start_gate {
                        logged_start_gate = true;
                        info!(
                            "mix startup trace: start gate satisfied at {}ms",
                            startup_trace.elapsed().as_millis()
                        );
                    }
                } else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }

            let mut samples_for_processing = if convolution_batch_samples > 0
                && pending_mix_samples.len() >= convolution_batch_samples
            {
                Some(
                    pending_mix_samples
                        .drain(0..convolution_batch_samples)
                        .collect::<Vec<f32>>(),
                )
            } else {
                None
            };

            if samples_for_processing.is_none() {
                if let Some(samples) = buffer_mixer.take_samples() {
                    if !logged_first_take_samples {
                        logged_first_take_samples = true;
                        info!(
                            "mix startup trace: first take_samples at {}ms (samples={})",
                            startup_trace.elapsed().as_millis(),
                            samples.len()
                        );
                    }
                    // info!("Took {} samples!", samples.len());
                    if convolution_batch_samples > 0 {
                        pending_mix_samples.extend_from_slice(&samples);
                        if pending_mix_samples.len() >= convolution_batch_samples {
                            samples_for_processing = Some(
                                pending_mix_samples
                                    .drain(0..convolution_batch_samples)
                                    .collect::<Vec<f32>>(),
                            );
                        }
                    } else {
                        samples_for_processing = Some(samples);
                    }
                }
            }

            if samples_for_processing.is_none()
                && buffer_mixer.mix_finished()
                && !pending_mix_samples.is_empty()
            {
                let missing_samples = convolution_batch_samples - pending_mix_samples.len();

                // Fill up pending_mix to full length
                pending_mix_samples.append(&mut vec![0.0; missing_samples]);
                samples_for_processing = Some(std::mem::take(&mut pending_mix_samples));
            }

            if let Some(samples) = samples_for_processing {
                running_count += samples.len();
                debug!("Processed {} samples so far!", running_count);
                if samples.len() < convolution_batch_samples {
                    warn!(
                        "Only processing {} samples! (Convolution wants {})",
                        samples.len(),
                        convolution_batch_samples
                    )
                };
                #[cfg(feature = "debug")]
                let audio_time_ms = if audio_info.channels > 0 && audio_info.sample_rate > 0 {
                    (samples.len() as f64
                        / audio_info.channels as f64
                        / audio_info.sample_rate as f64)
                        * 1000.0
                } else {
                    0.0
                };
                #[cfg(feature = "debug")]
                let dsp_start = Instant::now();
                let processed = if let Some(transition) = active_inline_transition.as_mut() {
                    let old_out = run_effect_chain(
                        &mut transition.old_effects,
                        &samples,
                        &effect_context,
                        false,
                    );
                    let new_out = run_effect_chain(
                        &mut transition.new_effects,
                        &samples,
                        &effect_context,
                        false,
                    );
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
                    transition.remaining_samples = transition
                        .remaining_samples
                        .saturating_sub(samples.len().max(1));
                    if transition.remaining_samples == 0 {
                        let mut effects_guard = effects.lock().unwrap();
                        *effects_guard = transition.new_effects.clone();
                        active_inline_transition = None;
                    }
                    blended
                } else {
                    let mut effects_guard = effects.lock().unwrap();
                    run_effect_chain(&mut effects_guard, &samples, &effect_context, false)
                };

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

                    if overrun_ms > 0.0 {
                        max_overrun_ms = max_overrun_ms.max(overrun_ms);
                    }
                    if chain_ksps > 0.0 {
                        min_chain_ksps = min_chain_ksps.min(chain_ksps);
                        max_chain_ksps = max_chain_ksps.max(chain_ksps);
                    }

                    if let Ok(mut metrics) = dsp_metrics.lock() {
                        metrics.overrun = overrun;
                        metrics.overrun_ms = overrun_ms;
                        metrics.avg_overrun_ms = avg_overrun_ms;
                        metrics.max_overrun_ms = max_overrun_ms;
                        metrics.chain_ksps = chain_ksps;
                        metrics.avg_chain_ksps = avg_chain_ksps;
                        metrics.min_chain_ksps = if min_chain_ksps.is_finite() {
                            min_chain_ksps
                        } else {
                            0.0
                        };
                        metrics.max_chain_ksps = max_chain_ksps;
                    }
                }

                let input_channels = audio_info.channels as u16;
                let sample_rate = audio_info.sample_rate;
                match super::output_stage::send_samples(
                    &sender,
                    input_channels,
                    sample_rate,
                    processed,
                ) {
                    super::output_stage::SendStatus::Sent => {
                        if !logged_first_output_send {
                            logged_first_output_send = true;
                            info!(
                                "mix startup trace: first output chunk sent at {}ms (processed_samples={})",
                                startup_trace.elapsed().as_millis(),
                                samples.len()
                            );
                        }
                        buffer_notify.notify_all();
                    }
                    super::output_stage::SendStatus::Empty => {}
                    super::output_stage::SendStatus::Disconnected => {
                        abort.store(true, Ordering::SeqCst);
                        break;
                    }
                }

                if let Ok(mut metrics) = dsp_metrics.lock() {
                    metrics.track_key_count = buffer_mixer.instance_count();
                    metrics.prot_key_count = buffer_mixer.logical_track_count();
                    metrics.finished_track_count = buffer_mixer.finished_instance_count();
                }
            } else if buffer_mixer.mix_finished() {
                #[cfg(feature = "debug")]
                pivot_buffer();
                info!("Mix Finished!!! (in runner)");
                effect_drain_passes = effect_drain_passes.saturating_add(1);
                if effect_drain_passes > MAX_EFFECT_DRAIN_PASSES {
                    warn!(
                        "effect drain stopped after {} passes to avoid infinite tail generation",
                        MAX_EFFECT_DRAIN_PASSES
                    );
                    break;
                }
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
                        blended.push((old_sample + new_sample) * 0.5);
                    }
                    blended
                } else {
                    let mut effects_guard = effects.lock().unwrap();
                    run_effect_chain(&mut effects_guard, &[], &effect_context, true)
                };

                if drained.is_empty() {
                    break;
                }

                let max_abs = drained
                    .iter()
                    .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
                if max_abs <= DRAIN_SILENCE_EPSILON {
                    effect_drain_silent_passes = effect_drain_silent_passes.saturating_add(1);
                } else {
                    effect_drain_silent_passes = 0;
                }
                if effect_drain_silent_passes >= DRAIN_SILENT_PASSES_TO_STOP {
                    info!("effect drain stopped after consecutive silent drain passes");
                    break;
                }

                let input_channels = audio_info.channels as u16;
                let sample_rate = audio_info.sample_rate;
                match super::output_stage::send_samples(
                    &sender,
                    input_channels,
                    sample_rate,
                    drained,
                ) {
                    super::output_stage::SendStatus::Sent => {}
                    super::output_stage::SendStatus::Empty => break,
                    super::output_stage::SendStatus::Disconnected => {
                        abort.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(2));
            }
        }

        let mut finished = finished_tracks.lock().unwrap();
        finished.clear();
        for index in 0..buffer_mixer.instance_count() {
            finished.push(index as u16);
        }

        decode_backpressure.shutdown();
        // Drop the packet receiver before joining decode workers so any worker
        // blocked on `packet_tx.send(...)` wakes with `SendError` instead of
        // deadlocking teardown during seek/stop.
        drop(packet_rx);
        drop(decode_workers);
    });

    (receiver, handle)
}

fn drain_decode_events(
    packet_rx: &mpsc::Receiver<DecodeWorkerEvent>,
    buffer_mixer: &mut BufferMixer,
    startup_trace: Instant,
    logged_first_packet_drain: &mut bool,
    logged_first_packet_route: &mut bool,
) {
    while let Ok(event) = packet_rx.try_recv() {
        if !*logged_first_packet_drain {
            info!(
                "mix startup trace: first packet dequeued from decode channel at {}ms",
                startup_trace.elapsed().as_millis()
            );
            *logged_first_packet_drain = true;
        }
        match event {
            DecodeWorkerEvent::Packet(packet) => {
                if !*logged_first_packet_route {
                    info!(
                        "mix startup trace: first packet route start at {}ms (source={:?} ts={:.6} samples={})",
                        startup_trace.elapsed().as_millis(),
                        packet.source_key,
                        packet.packet_ts,
                        packet.samples.len()
                    );
                    *logged_first_packet_route = true;
                }
                let _decision =
                    buffer_mixer.route_packet(&packet.samples, packet.source_key, packet.packet_ts);
            }
            DecodeWorkerEvent::SourceFinished { source_key } => {
                buffer_mixer.signal_finish(&source_key);
            }
            DecodeWorkerEvent::SourceError {
                source_key,
                recoverable,
                message,
            } => {
                if recoverable {
                    warn!(
                        "decode worker recoverable error: source={:?} {}",
                        source_key, message
                    );
                } else {
                    warn!(
                        "decode worker terminal error: source={:?} {}",
                        source_key, message
                    );
                    buffer_mixer.signal_finish(&source_key);
                }
            }
            DecodeWorkerEvent::StreamExhausted => {
                buffer_mixer.signal_finish_all();
            }
        }
    }
}

fn apply_inline_track_mix_updates(
    inline_track_mix_updates: &Arc<Mutex<Vec<crate::playback::engine::InlineTrackMixUpdate>>>,
    buffer_mixer: &mut BufferMixer,
) {
    let updates = {
        let mut pending = inline_track_mix_updates.lock().unwrap();
        std::mem::take(&mut *pending)
    };
    for update in updates {
        buffer_mixer.set_track_mix_by_slot(update.slot_index, update.level, update.pan);
    }
}

fn apply_effect_runtime_updates(
    effects_reset: &Arc<std::sync::atomic::AtomicU64>,
    last_effects_reset: &mut u64,
    effects: &Arc<Mutex<Vec<AudioEffect>>>,
    active_inline_transition: &mut Option<ActiveInlineTransition>,
    inline_effects_update: &Arc<Mutex<Option<crate::playback::engine::InlineEffectsUpdate>>>,
    prot_locked: &Arc<Mutex<crate::container::prot::Prot>>,
    audio_info: &crate::container::info::Info,
    effect_context: &mut EffectContext,
) {
    let current_reset = effects_reset.load(Ordering::SeqCst);
    if current_reset != *last_effects_reset {
        let mut effects_guard = effects.lock().unwrap();
        for effect in effects_guard.iter_mut() {
            effect.reset_state();
        }
        *active_inline_transition = None;
        inline_effects_update.lock().unwrap().take();
        *effect_context = {
            let prot = prot_locked.lock().unwrap();
            EffectContext {
                sample_rate: prot.info.sample_rate,
                channels: prot.info.channels as usize,
                container_path: prot.get_container_path(),
                impulse_response_spec: prot.get_impulse_response_spec(),
                impulse_response_tail_db: prot.get_impulse_response_tail_db().unwrap_or(-60.0),
            }
        };
        *last_effects_reset = current_reset;
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
                effect.warm_up(effect_context);
            }
            *active_inline_transition = None;
        } else {
            let old_effects = {
                let effects_guard = effects.lock().unwrap();
                effects_guard.clone()
            };
            let mut new_effects = update.effects;
            for effect in new_effects.iter_mut() {
                effect.warm_up(effect_context);
            }
            *active_inline_transition = Some(ActiveInlineTransition {
                old_effects,
                new_effects,
                total_samples: transition_samples,
                remaining_samples: transition_samples,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_constants_are_positive() {
        assert!(MAX_EFFECT_DRAIN_PASSES > 0);
        assert!(DRAIN_SILENT_PASSES_TO_STOP > 0);
        assert!(DRAIN_SILENCE_EPSILON > 0.0);
    }
}
