//! Mix loop state and constructor.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc, Condvar, Mutex};

use rodio::buffer::SamplesBuffer;

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::{AudioEffect, EffectContext};
use crate::playback::engine::{DspChainMetrics, InlineEffectsUpdate, InlineTrackMixUpdate};

use super::super::buffer_mixer::{BufferMixer, DecodeBackpressure};
use super::super::decoder_events::DecodeWorkerEvent;
use super::super::types::{ActiveInlineTransition, MixThreadArgs};
use super::decode::DecodeWorkerJoinGuard;

/// Precomputed mixing buffer sizes.
pub(super) struct MixBufferSizes {
    pub start_samples: usize,
    pub min_mix_samples: usize,
    pub convolution_batch_samples: usize,
}

/// Decode infrastructure built during startup.
pub(super) struct MixDecodeHandle {
    pub decode_backpressure: Arc<DecodeBackpressure>,
    pub packet_rx: mpsc::Receiver<DecodeWorkerEvent>,
    pub decode_workers: DecodeWorkerJoinGuard,
}

pub(super) struct MixLoopState {
    pub(super) abort: Arc<AtomicBool>,
    pub(super) packet_rx: mpsc::Receiver<DecodeWorkerEvent>,
    pub(super) buffer_mixer: BufferMixer,
    pub(super) decode_backpressure: Arc<DecodeBackpressure>,
    pub(super) effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub(super) effect_context: EffectContext,
    pub(super) sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    pub(super) buffer_notify: Arc<Condvar>,
    pub(super) audio_info: Info,
    pub(super) dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    pub(super) inline_track_mix_updates: Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    pub(super) inline_effects_update: Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub(super) effects_reset: Arc<AtomicU64>,
    pub(super) prot: Arc<Mutex<Prot>>,
    pub(super) finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub(super) convolution_batch_samples: usize,
    pub(super) start_samples: usize,
    pub(super) min_mix_samples: usize,
    pub(super) started: bool,
    pub(super) last_effects_reset: u64,
    pub(super) active_inline_transition: Option<ActiveInlineTransition>,
    pub(super) pending_mix_samples: Vec<f32>,
    pub(super) effect_drain_passes: usize,
    pub(super) effect_drain_silent_passes: usize,
    pub(super) running_count: usize,
    pub(super) logged_first_packet_drain: bool,
    pub(super) logged_first_packet_route: bool,
    pub(super) logged_start_gate: bool,
    pub(super) logged_first_take_samples: bool,
    pub(super) logged_first_output_send: bool,
    pub(super) decode_workers: DecodeWorkerJoinGuard,
    #[cfg(feature = "debug")]
    pub(super) alpha: f64,
    #[cfg(feature = "debug")]
    pub(super) avg_overrun_ms: f64,
    #[cfg(feature = "debug")]
    pub(super) max_overrun_ms: f64,
    #[cfg(feature = "debug")]
    pub(super) avg_chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub(super) min_chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub(super) max_chain_ksps: f64,
}

impl MixLoopState {
    pub(super) fn new(
        args: MixThreadArgs,
        sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
        buffer_mixer: BufferMixer,
        effect_context: EffectContext,
        sizes: MixBufferSizes,
        decode_handle: MixDecodeHandle,
    ) -> Self {
        let last_effects_reset = args.effects_reset.load(std::sync::atomic::Ordering::SeqCst);
        let start_samples = sizes.start_samples;
        Self {
            abort: args.abort,
            packet_rx: decode_handle.packet_rx,
            buffer_mixer,
            decode_backpressure: decode_handle.decode_backpressure,
            effects: args.effects,
            effect_context,
            sender,
            buffer_notify: args.buffer_notify,
            audio_info: args.audio_info,
            dsp_metrics: args.dsp_metrics,
            inline_track_mix_updates: args.inline_track_mix_updates,
            inline_effects_update: args.inline_effects_update,
            effects_reset: args.effects_reset,
            prot: args.prot,
            finished_tracks: args.finished_tracks,
            convolution_batch_samples: sizes.convolution_batch_samples,
            start_samples,
            min_mix_samples: sizes.min_mix_samples,
            started: start_samples == 0,
            last_effects_reset,
            active_inline_transition: None,
            pending_mix_samples: Vec::new(),
            effect_drain_passes: 0,
            effect_drain_silent_passes: 0,
            running_count: 0,
            logged_first_packet_drain: false,
            logged_first_packet_route: false,
            logged_start_gate: false,
            logged_first_take_samples: false,
            logged_first_output_send: false,
            decode_workers: decode_handle.decode_workers,
            #[cfg(feature = "debug")]
            alpha: 0.1,
            #[cfg(feature = "debug")]
            avg_overrun_ms: 0.0,
            #[cfg(feature = "debug")]
            max_overrun_ms: 0.0,
            #[cfg(feature = "debug")]
            avg_chain_ksps: 0.0,
            #[cfg(feature = "debug")]
            min_chain_ksps: f64::INFINITY,
            #[cfg(feature = "debug")]
            max_chain_ksps: 0.0,
        }
    }
}
