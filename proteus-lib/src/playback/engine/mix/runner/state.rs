//! Mix loop state and constructor.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};

use rodio::buffer::SamplesBuffer;

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::{AudioEffect, EffectContext};
use crate::playback::engine::premix::PremixBuffer;
use crate::playback::engine::{
    DspChainMetrics, InlineEffectsUpdate, InlineTrackMixUpdate, PlaybackBufferSettings,
};
use crate::playback::mutex_policy::lock_recoverable;

use super::super::buffer_mixer::{BufferMixer, DecodeBackpressure};
use super::super::effects::EffectEnableFade;
use super::super::decoder_events::DecodeWorkerEvent;
use super::super::types::{ActiveInlineTransition, EffectSettingsCommand, MixThreadArgs};
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
    pub(super) local_effects: Vec<AudioEffect>,
    pub(super) effect_settings_commands: Arc<Mutex<Vec<EffectSettingsCommand>>>,
    pub(super) effect_context: EffectContext,
    pub(super) sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    pub(super) buffer_notify: Arc<Condvar>,
    pub(super) audio_info: Info,
    pub(super) buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
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
    pub(super) pending_mix_samples: PremixBuffer,
    pub(super) effect_enable_fades: Vec<Option<EffectEnableFade>>,
    pub(super) effect_scratch_a: Vec<f32>,
    pub(super) effect_scratch_b: Vec<f32>,
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
        let local_effects = lock_recoverable(
            &args.effects,
            "mix runtime effects",
            "the effect chain is hot-swappable runtime state",
        )
        .clone();
        let effect_count = local_effects.len();
        Self {
            abort: args.abort,
            packet_rx: decode_handle.packet_rx,
            buffer_mixer,
            decode_backpressure: decode_handle.decode_backpressure,
            effects: args.effects,
            local_effects,
            effect_settings_commands: args.effect_settings_commands,
            effect_context,
            sender,
            buffer_notify: args.buffer_notify,
            audio_info: args.audio_info,
            buffer_settings: args.buffer_settings,
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
            pending_mix_samples: PremixBuffer::new(),
            effect_enable_fades: vec![None; effect_count],
            effect_scratch_a: Vec::new(),
            effect_scratch_b: Vec::new(),
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

    /// Recoverable poison policy: the effect chain is hot-swappable runtime state.
    pub(super) fn lock_effects_recoverable(&self) -> MutexGuard<'_, Vec<AudioEffect>> {
        lock_recoverable(
            &self.effects,
            "mix runtime effects",
            "the effect chain is hot-swappable runtime state",
        )
    }

    /// Recoverable poison policy: DSP metrics are derived telemetry.
    pub(super) fn lock_dsp_metrics_recoverable(&self) -> MutexGuard<'_, DspChainMetrics> {
        lock_recoverable(
            &self.dsp_metrics,
            "mix runtime DSP metrics",
            "DSP metrics are derived telemetry that can be rebuilt",
        )
    }

    /// Recoverable poison policy: buffer settings are runtime configuration snapshots.
    pub(super) fn lock_buffer_settings_recoverable(
        &self,
    ) -> MutexGuard<'_, PlaybackBufferSettings> {
        lock_recoverable(
            &self.buffer_settings,
            "mix runtime buffer settings",
            "buffer settings are runtime configuration snapshots",
        )
    }

    /// Recoverable poison policy: pending inline effect updates are a disposable queue.
    pub(super) fn lock_inline_effects_update_recoverable(
        &self,
    ) -> MutexGuard<'_, Option<InlineEffectsUpdate>> {
        lock_recoverable(
            &self.inline_effects_update,
            "mix runtime inline effects update",
            "pending inline effect updates are a disposable queue",
        )
    }

    /// Recoverable poison policy: effect settings commands are a disposable control queue.
    pub(super) fn lock_effect_settings_commands_recoverable(
        &self,
    ) -> MutexGuard<'_, Vec<EffectSettingsCommand>> {
        lock_recoverable(
            &self.effect_settings_commands,
            "mix runtime effect settings commands",
            "incremental effect settings commands are a disposable control queue",
        )
    }

    /// Recoverable poison policy: finished-track bookkeeping is rebuildable runtime state.
    pub(super) fn lock_finished_tracks_recoverable(&self) -> MutexGuard<'_, Vec<u16>> {
        lock_recoverable(
            &self.finished_tracks,
            "mix runtime finished tracks",
            "finished-track bookkeeping is rebuildable runtime state",
        )
    }
}
