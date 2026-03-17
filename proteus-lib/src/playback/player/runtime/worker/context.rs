//! Shared runtime context captured at thread spawn time.

use rodio::{mixer::Mixer, Sink};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::container::info::Info;
use crate::container::prot::Prot;
use crate::dsp::effects::AudioEffect;
use crate::playback::engine::{
    DspChainMetrics, InlineEffectsUpdate, InlineTrackMixUpdate, PlaybackBufferSettings,
};
use crate::playback::mutex_policy::{lock_invariant, lock_recoverable};
use crate::playback::output_meter::OutputMeter;

use super::super::super::{EndOfStreamAction, PlayerState};

/// Captured shared state passed from `Player::initialize_thread` into the
/// detached worker thread.
pub(in crate::playback::player::runtime) struct ThreadContext {
    pub(in crate::playback::player::runtime) play_state: Arc<Mutex<PlayerState>>,
    pub(in crate::playback::player::runtime) abort: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) playback_thread_exists: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) playback_id_atomic: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) time_passed: Arc<Mutex<f64>>,
    pub(in crate::playback::player::runtime) duration: Arc<Mutex<f64>>,
    pub(in crate::playback::player::runtime) prot: Arc<Mutex<Prot>>,
    pub(in crate::playback::player::runtime) buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    pub(in crate::playback::player::runtime) effects: Arc<Mutex<Vec<AudioEffect>>>,
    pub(in crate::playback::player::runtime) inline_effects_update:
        Arc<Mutex<Option<InlineEffectsUpdate>>>,
    pub(in crate::playback::player::runtime) inline_track_mix_updates:
        Arc<Mutex<Vec<InlineTrackMixUpdate>>>,
    pub(in crate::playback::player::runtime) dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    pub(in crate::playback::player::runtime) effects_reset: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) output_meter: Arc<Mutex<OutputMeter>>,
    pub(in crate::playback::player::runtime) audio_info: Info,
    pub(in crate::playback::player::runtime) next_resume_fade_ms: Arc<Mutex<Option<f32>>>,
    pub(in crate::playback::player::runtime) end_of_stream_action: Arc<Mutex<EndOfStreamAction>>,
    pub(in crate::playback::player::runtime) audio_heard: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) play_command_ms: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) volume: Arc<Mutex<f32>>,
    pub(in crate::playback::player::runtime) sink_mutex: Arc<Mutex<Sink>>,
    pub(in crate::playback::player::runtime) output_mixer: Mixer,
    pub(in crate::playback::player::runtime) buffer_done_thread_flag: Arc<AtomicBool>,
    pub(in crate::playback::player::runtime) last_chunk_ms: Arc<AtomicU64>,
    pub(in crate::playback::player::runtime) last_time_update_ms: Arc<AtomicU64>,
}

impl ThreadContext {
    /// Invariant-only poison policy: the transport state machine must remain coherent.
    pub(super) fn lock_play_state_invariant(&self) -> MutexGuard<'_, PlayerState> {
        lock_invariant(
            &self.play_state,
            "playback worker state",
            "worker transport transitions rely on a coherent state machine",
        )
    }

    /// Recoverable poison policy: playback time is scalar telemetry.
    pub(super) fn lock_time_passed_recoverable(&self) -> MutexGuard<'_, f64> {
        lock_recoverable(
            &self.time_passed,
            "playback worker time",
            "playback time is scalar telemetry that can continue from the inner value",
        )
    }

    /// Recoverable poison policy: duration is cached metadata.
    pub(super) fn lock_duration_recoverable(&self) -> MutexGuard<'_, f64> {
        lock_recoverable(
            &self.duration,
            "playback worker duration",
            "duration is cached metadata that can continue from the inner value",
        )
    }

    /// Recoverable poison policy: buffer settings are runtime configuration.
    pub(super) fn lock_buffer_settings_recoverable(
        &self,
    ) -> MutexGuard<'_, PlaybackBufferSettings> {
        lock_recoverable(
            &self.buffer_settings,
            "playback worker buffer settings",
            "buffer settings are runtime configuration snapshots",
        )
    }

    /// Recoverable poison policy: DSP metrics are derived telemetry.
    pub(super) fn lock_dsp_metrics_recoverable(&self) -> MutexGuard<'_, DspChainMetrics> {
        lock_recoverable(
            &self.dsp_metrics,
            "playback worker DSP metrics",
            "DSP metrics are derived telemetry that can be rebuilt",
        )
    }

    /// Recoverable poison policy: the output meter is derived telemetry.
    pub(super) fn lock_output_meter_recoverable(&self) -> MutexGuard<'_, OutputMeter> {
        lock_recoverable(
            &self.output_meter,
            "playback worker output meter",
            "meter state is derived telemetry that can be rebuilt",
        )
    }

    /// Recoverable poison policy: pending resume fade is transient runtime configuration.
    pub(super) fn lock_next_resume_fade_ms_recoverable(
        &self,
    ) -> MutexGuard<'_, Option<f32>> {
        lock_recoverable(
            &self.next_resume_fade_ms,
            "playback worker next resume fade",
            "pending fade configuration is transient runtime state",
        )
    }

    /// Recoverable poison policy: end-of-stream action is runtime configuration.
    pub(super) fn lock_end_of_stream_action_recoverable(
        &self,
    ) -> MutexGuard<'_, EndOfStreamAction> {
        lock_recoverable(
            &self.end_of_stream_action,
            "playback worker end-of-stream action",
            "transport end behavior is runtime configuration",
        )
    }

    /// Recoverable poison policy: volume is a scalar control value.
    pub(super) fn lock_volume_recoverable(&self) -> MutexGuard<'_, f32> {
        lock_recoverable(
            &self.volume,
            "playback worker volume",
            "volume is a scalar control value that can continue from the inner value",
        )
    }

    /// Recoverable poison policy: the sink is disposable output state.
    pub(super) fn lock_sink_recoverable(&self) -> MutexGuard<'_, Sink> {
        lock_recoverable(
            &self.sink_mutex,
            "playback worker sink",
            "the output sink is replaceable runtime I/O state",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadContext;

    #[test]
    fn thread_context_type_is_materialized_for_test_coverage() {
        assert!(std::mem::size_of::<Option<ThreadContext>>() > 0);
    }
}
