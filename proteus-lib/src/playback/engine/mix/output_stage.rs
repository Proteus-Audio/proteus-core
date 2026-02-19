//! Output-stage DSP helpers for the mix runtime.

use dasp_ring_buffer::Bounded;
use rodio::buffer::SamplesBuffer;
use std::sync::{mpsc, Arc, Mutex};
#[cfg(feature = "debug")]
use std::time::Instant;

use crate::dsp::effects::{AudioEffect, EffectContext};

use super::super::premix::PremixBuffer;
#[cfg(feature = "debug")]
use super::debug::effect_label;
use super::effects::run_effect_chain;
use super::types::ActiveInlineTransition;

/// Arguments for producing one output sample block from premix/tail state.
pub(super) struct OutputStageArgs<'a> {
    pub(super) effects_buffer: &'a Arc<Mutex<Bounded<Vec<f32>>>>,
    pub(super) premix_buffer: &'a mut PremixBuffer,
    pub(super) min_mix_samples: usize,
    pub(super) all_tracks_finished: bool,
    pub(super) input_channels: u16,
    pub(super) sample_rate: u32,
    pub(super) effect_context: &'a EffectContext,
    pub(super) active_inline_transition: &'a mut Option<ActiveInlineTransition>,
    pub(super) effects: &'a Arc<Mutex<Vec<AudioEffect>>>,
    pub(super) buffer_settings: &'a Arc<Mutex<crate::playback::engine::PlaybackBufferSettings>>,
    pub(super) dsp_metrics: &'a Arc<Mutex<crate::playback::engine::DspChainMetrics>>,
    pub(super) underrun_count: u64,
    pub(super) track_key_count: usize,
    pub(super) finished_track_count: usize,
    pub(super) prot_key_count: usize,
    #[cfg(feature = "debug")]
    pub(super) avg_overrun_ms: &'a mut f64,
    #[cfg(feature = "debug")]
    pub(super) avg_chain_ksps: &'a mut f64,
    #[cfg(feature = "debug")]
    pub(super) min_chain_ksps: &'a mut f64,
    #[cfg(feature = "debug")]
    pub(super) max_chain_ksps: &'a mut f64,
    #[cfg(feature = "debug")]
    pub(super) max_overrun_ms: &'a mut f64,
    #[cfg(feature = "debug")]
    pub(super) pop_count: &'a mut u64,
    #[cfg(feature = "debug")]
    pub(super) clip_count: &'a mut u64,
    #[cfg(feature = "debug")]
    pub(super) nan_count: &'a mut u64,
    #[cfg(feature = "debug")]
    pub(super) last_pop_log: &'a mut Instant,
    #[cfg(feature = "debug")]
    pub(super) last_boundary_log: &'a mut Instant,
    #[cfg(feature = "debug")]
    pub(super) last_samples: &'a mut Vec<f32>,
    #[cfg(feature = "debug")]
    pub(super) boundary_initialized: &'a mut bool,
    #[cfg(feature = "debug")]
    pub(super) boundary_count: &'a mut u64,
    #[cfg(feature = "debug")]
    pub(super) effect_boundary_initialized: &'a mut Vec<bool>,
    #[cfg(feature = "debug")]
    pub(super) effect_last_samples: &'a mut Vec<Vec<f32>>,
    #[cfg(feature = "debug")]
    pub(super) effect_boundary_counts: &'a mut Vec<u64>,
    #[cfg(feature = "debug")]
    pub(super) effect_boundary_logs: &'a mut Vec<Instant>,
    #[cfg(feature = "debug")]
    pub(super) alpha: f64,
}

/// Produce one output sample block from tail or premix data.
pub(super) fn produce_output_samples(args: OutputStageArgs<'_>) -> Vec<f32> {
    let OutputStageArgs {
        effects_buffer,
        premix_buffer,
        min_mix_samples,
        all_tracks_finished,
        input_channels,
        sample_rate,
        effect_context,
        active_inline_transition,
        effects,
        buffer_settings,
        dsp_metrics,
        underrun_count,
        track_key_count,
        finished_track_count,
        prot_key_count,
        #[cfg(feature = "debug")]
        avg_overrun_ms,
        #[cfg(feature = "debug")]
        avg_chain_ksps,
        #[cfg(feature = "debug")]
        min_chain_ksps,
        #[cfg(feature = "debug")]
        max_chain_ksps,
        #[cfg(feature = "debug")]
        max_overrun_ms,
        #[cfg(feature = "debug")]
        pop_count,
        #[cfg(feature = "debug")]
        clip_count,
        #[cfg(feature = "debug")]
        nan_count,
        #[cfg(feature = "debug")]
        last_pop_log,
        #[cfg(feature = "debug")]
        last_boundary_log,
        #[cfg(feature = "debug")]
        last_samples,
        #[cfg(feature = "debug")]
        boundary_initialized,
        #[cfg(feature = "debug")]
        boundary_count,
        #[cfg(feature = "debug")]
        effect_boundary_initialized,
        #[cfg(feature = "debug")]
        effect_last_samples,
        #[cfg(feature = "debug")]
        effect_boundary_counts,
        #[cfg(feature = "debug")]
        effect_boundary_logs,
        #[cfg(feature = "debug")]
        alpha,
    } = args;

    #[cfg(not(feature = "debug"))]
    let _ = (
        input_channels,
        sample_rate,
        buffer_settings,
        dsp_metrics,
        underrun_count,
        track_key_count,
        finished_track_count,
        prot_key_count,
    );

    let effects_len = effects_buffer.lock().unwrap().len();
    let should_output_tail = effects_len > 0;
    let should_process_premix = !should_output_tail
        && (premix_buffer.len() >= min_mix_samples
            || (all_tracks_finished && !premix_buffer.is_empty()));

    if should_output_tail {
        let mut tail_buffer = effects_buffer.lock().unwrap();
        let take = tail_buffer.len().min(min_mix_samples).max(1);
        let mut out = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(sample) = tail_buffer.pop() {
                out.push(sample);
            }
        }
        return out;
    }

    if !should_process_premix {
        return Vec::new();
    }

    let take = if premix_buffer.len() >= min_mix_samples {
        min_mix_samples
    } else {
        premix_buffer.len()
    };
    let dsp_input = premix_buffer.pop_chunk(take);

    #[cfg(feature = "debug")]
    let audio_time_ms = if input_channels > 0 && sample_rate > 0 {
        let frames = dsp_input.len() as f64 / input_channels as f64;
        (frames / sample_rate as f64) * 1000.0
    } else {
        0.0
    };

    #[cfg(feature = "debug")]
    let dsp_start = Instant::now();

    let drain_effects = all_tracks_finished && premix_buffer.is_empty();
    let mut processed = if let Some(transition) = active_inline_transition.as_mut() {
        let old_out = run_effect_chain(
            &mut transition.old_effects,
            &dsp_input,
            effect_context,
            drain_effects,
        );
        let new_out = run_effect_chain(
            &mut transition.new_effects,
            &dsp_input,
            effect_context,
            drain_effects,
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
            if transition.remaining_samples > 0 {
                transition.remaining_samples -= 1;
            }
        }

        if transition.remaining_samples == 0 {
            let mut effects_guard = effects.lock().unwrap();
            *effects_guard = std::mem::take(&mut transition.new_effects);
            *active_inline_transition = None;
        }
        blended
    } else {
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
                *effect_boundary_initialized = vec![false; effect_count];
                *effect_last_samples = vec![vec![0.0; channels]; effect_count];
                *effect_boundary_counts = vec![0_u64; effect_count];
                *effect_boundary_logs = vec![Instant::now(); effect_count];
            }
        }

        let mut current = dsp_input.clone();
        for (effect_index, effect) in effects_guard.iter_mut().enumerate() {
            #[cfg(not(feature = "debug"))]
            let _ = effect_index;
            current = effect.process(&current, effect_context, drain_effects);
            #[cfg(feature = "debug")]
            if effect_boundary_log {
                let channels = input_channels.max(1) as usize;
                if effect_index < effect_last_samples.len()
                    && effect_index < effect_boundary_initialized.len()
                    && current.len() >= channels
                {
                    let initialized = effect_boundary_initialized[effect_index];
                    for ch in 0..channels {
                        let prev = effect_last_samples[effect_index][ch];
                        let curr = current[ch];
                        let delta = (curr - prev).abs();
                        if initialized && delta > 0.1 {
                            effect_boundary_counts[effect_index] =
                                effect_boundary_counts[effect_index].saturating_add(1);
                            if effect_boundary_logs[effect_index].elapsed().as_millis() >= 200 {
                                log::info!(
                                    "effect boundary discontinuity: effect={} delta={:.4} prev={:.4} curr={:.4} ch={} count={}",
                                    effect_label(effect),
                                    delta,
                                    prev,
                                    curr,
                                    ch,
                                    effect_boundary_counts[effect_index]
                                );
                                effect_boundary_logs[effect_index] = Instant::now();
                            }
                        }
                    }
                    let last_frame_start = current.len().saturating_sub(channels);
                    for ch in 0..channels {
                        let idx = (last_frame_start + ch).min(current.len().saturating_sub(1));
                        effect_last_samples[effect_index][ch] = current[idx];
                    }
                    if !effect_boundary_initialized[effect_index] && !current.is_empty() {
                        effect_boundary_initialized[effect_index] = true;
                    }
                }
            }
        }
        current
    };

    if processed.len() < dsp_input.len() {
        let missing = dsp_input.len().saturating_sub(processed.len());
        let start = dsp_input.len().saturating_sub(missing);
        processed.extend_from_slice(&dsp_input[start..]);
    } else if processed.len() > dsp_input.len() {
        let extra = processed.split_off(dsp_input.len());
        let mut tail_buffer = effects_buffer.lock().unwrap();
        for sample in extra {
            let _ = tail_buffer.push(sample);
        }
    }

    #[cfg(feature = "debug")]
    {
        let channels = input_channels.max(1) as usize;
        if last_samples.len() != channels {
            *last_samples = vec![0.0; channels];
        }
        for (idx, sample) in processed.iter().enumerate() {
            let ch = idx % channels;
            let prev = last_samples[ch];
            if sample.is_nan() || sample.is_infinite() {
                *nan_count = nan_count.saturating_add(1);
            }
            if sample.abs() > 1.0 {
                *clip_count = clip_count.saturating_add(1);
            }
            let delta = (sample - prev).abs();
            if *boundary_initialized && idx < channels && delta > 0.1 {
                *boundary_count = boundary_count.saturating_add(1);
                if last_boundary_log.elapsed().as_millis() >= 200 {
                    log::info!(
                        "boundary discontinuity: delta={:.4} prev={:.4} curr={:.4} ch={} count={}",
                        delta,
                        prev,
                        sample,
                        ch,
                        boundary_count
                    );
                    *last_boundary_log = Instant::now();
                }
            }
            if delta > 0.9 && sample.abs() > 0.6 {
                *pop_count = pop_count.saturating_add(1);
            }
            last_samples[ch] = *sample;
        }
        if !*boundary_initialized && !processed.is_empty() {
            *boundary_initialized = true;
        }
        if last_pop_log.elapsed().as_secs_f64() >= 1.0
            && (*pop_count > 0 || *clip_count > 0 || *nan_count > 0)
        {
            log::warn!(
                "sample anomalies: pops={} clips={} nans={}",
                pop_count,
                clip_count,
                nan_count
            );
            *last_pop_log = Instant::now();
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

        *avg_overrun_ms = if *avg_overrun_ms == 0.0 {
            overrun_ms
        } else {
            (*avg_overrun_ms * (1.0 - alpha)) + (overrun_ms * alpha)
        };
        *avg_chain_ksps = if *avg_chain_ksps == 0.0 {
            chain_ksps
        } else {
            (*avg_chain_ksps * (1.0 - alpha)) + (chain_ksps * alpha)
        };

        if overrun_ms > 0.0 {
            *max_overrun_ms = max_overrun_ms.max(overrun_ms);
        }
        if chain_ksps > 0.0 {
            *min_chain_ksps = min_chain_ksps.min(chain_ksps);
            *max_chain_ksps = max_chain_ksps.max(chain_ksps);
        }

        let mut metrics = dsp_metrics.lock().unwrap();
        metrics.overrun = overrun;
        metrics.overrun_ms = overrun_ms;
        metrics.avg_overrun_ms = *avg_overrun_ms;
        metrics.max_overrun_ms = *max_overrun_ms;
        metrics.chain_ksps = chain_ksps;
        metrics.avg_chain_ksps = *avg_chain_ksps;
        metrics.min_chain_ksps = if min_chain_ksps.is_finite() {
            *min_chain_ksps
        } else {
            0.0
        };
        metrics.max_chain_ksps = *max_chain_ksps;
        metrics.underrun_count = underrun_count;
        metrics.underrun_active = false;
        metrics.pop_count = *pop_count;
        metrics.clip_count = *clip_count;
        metrics.nan_count = *nan_count;
        metrics.track_key_count = track_key_count;
        metrics.finished_track_count = finished_track_count;
        metrics.prot_key_count = prot_key_count;
    }

    processed
}

/// Send produced samples over the mix thread output channel.
pub(super) enum SendStatus {
    Sent,
    Empty,
    Disconnected,
}

/// Send produced samples over the mix thread output channel.
pub(super) fn send_samples(
    sender: &mpsc::SyncSender<(SamplesBuffer, f64)>,
    input_channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
) -> SendStatus {
    if samples.is_empty() {
        return SendStatus::Empty;
    }

    let length_in_seconds = samples.len() as f64 / sample_rate as f64 / input_channels as f64;
    let samples_buffer = SamplesBuffer::new(input_channels, sample_rate, samples);

    if let Err(e) = sender.send((samples_buffer, length_in_seconds)) {
        log::error!("Failed to send samples: {}", e);
        return SendStatus::Disconnected;
    }
    SendStatus::Sent
}
