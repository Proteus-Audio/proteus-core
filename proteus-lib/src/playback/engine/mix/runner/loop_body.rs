//! Per-iteration helper functions for the mix-thread loop.

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};

use log::{info, warn};

use crate::dsp::effects::{AudioEffect, EffectContext};

use super::super::buffer_mixer::BufferMixer;
use super::super::decoder_events::DecodeWorkerEvent;
use super::super::types::ActiveInlineTransition;

/// Drain all pending decode worker events from the channel into the buffer mixer.
pub(super) fn drain_decode_events(
    packet_rx: &mpsc::Receiver<DecodeWorkerEvent>,
    buffer_mixer: &mut BufferMixer,
    startup_trace: std::time::Instant,
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

/// Flush pending inline track mix updates into the buffer mixer.
pub(super) fn apply_inline_track_mix_updates(
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

/// Apply effect resets and inline effect transitions for the current loop iteration.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_effect_runtime_updates(
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
