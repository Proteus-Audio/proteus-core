//! Centralized poison-policy accessors for critical `Player` mutexes.

use std::sync::MutexGuard;

use rodio::{OutputStream, Sink};

use super::{EndOfStreamAction, Player, PlayerState};
use crate::container::prot::Prot;
use crate::diagnostics::reporter::Reporter;
use crate::dsp::effects::AudioEffect;
use crate::playback::engine::{
    DspChainMetrics, EffectSettingsCommand, InlineEffectsUpdate, InlineTrackMixUpdate,
    PlaybackBufferSettings,
};
use crate::playback::mutex_policy::{lock_invariant, lock_recoverable};
use crate::playback::output_meter::OutputMeter;

impl Player {
    /// Recoverable poison policy: playback position is telemetry and can resume from the inner value.
    pub(in crate::playback::player) fn lock_ts_recoverable(&self) -> MutexGuard<'_, f64> {
        lock_recoverable(
            &self.ts,
            "player timestamp",
            "position tracking is a scalar snapshot that can continue from its last value",
        )
    }

    /// Invariant-only poison policy: transport transitions require a coherent state machine.
    pub(in crate::playback::player) fn lock_state_invariant(&self) -> MutexGuard<'_, PlayerState> {
        lock_invariant(
            &self.state,
            "player state",
            "transport transitions rely on a coherent state machine",
        )
    }

    /// Invariant-only poison policy: thread-handle ownership must stay consistent during joins.
    pub(in crate::playback::player) fn lock_playback_thread_handle_invariant(
        &self,
    ) -> MutexGuard<'_, Option<std::thread::JoinHandle<()>>> {
        lock_invariant(
            &self.playback_thread_handle,
            "playback thread handle",
            "join ownership cannot be reconstructed safely after a panic",
        )
    }

    /// Recoverable poison policy: duration is cached metadata and can continue from the inner value.
    pub(in crate::playback::player) fn lock_duration_recoverable(&self) -> MutexGuard<'_, f64> {
        lock_recoverable(
            &self.duration,
            "player duration",
            "duration is cached metadata that can continue from the inner value",
        )
    }

    /// Invariant-only poison policy: container mutations must not proceed from a potentially broken model.
    pub(in crate::playback::player) fn lock_prot_invariant(&self) -> MutexGuard<'_, Prot> {
        lock_invariant(
            &self.prot,
            "player prot",
            "container selection and effect metadata must stay internally consistent",
        )
    }

    /// Recoverable poison policy: volume is a scalar control value and can continue from the inner value.
    pub(in crate::playback::player) fn lock_volume_recoverable(&self) -> MutexGuard<'_, f32> {
        lock_recoverable(
            &self.volume,
            "player volume",
            "volume is a scalar control value that can continue from the inner value",
        )
    }

    /// Recoverable poison policy: the sink is disposable output state and should not cascade failures.
    pub(in crate::playback::player) fn lock_sink_recoverable(&self) -> MutexGuard<'_, Sink> {
        lock_recoverable(
            &self.sink,
            "player sink",
            "the output sink is replaceable runtime I/O state",
        )
    }

    /// Recoverable poison policy: the output stream handle can be reopened or reused from its inner value.
    pub(in crate::playback::player) fn lock_output_stream_recoverable(
        &self,
    ) -> MutexGuard<'_, Option<OutputStream>> {
        lock_recoverable(
            &self.output_stream,
            "player output stream",
            "the output stream handle is disposable runtime I/O state",
        )
    }

    /// Invariant-only poison policy: reporter lifecycle ownership must stay coherent.
    pub(in crate::playback::player) fn lock_reporter_invariant(
        reporter: &std::sync::Arc<std::sync::Mutex<Reporter>>,
    ) -> MutexGuard<'_, Reporter> {
        lock_invariant(
            reporter,
            "player reporter",
            "reporter thread lifecycle ownership must stay coherent",
        )
    }

    /// Recoverable poison policy: buffer settings are runtime configuration snapshots.
    pub(in crate::playback::player) fn lock_buffer_settings_recoverable(
        &self,
    ) -> MutexGuard<'_, PlaybackBufferSettings> {
        lock_recoverable(
            &self.buffer_settings,
            "player buffer settings",
            "buffer settings are runtime configuration snapshots",
        )
    }

    /// Recoverable poison policy: the effect chain is hot-swappable runtime state.
    pub(in crate::playback::player) fn lock_effects_recoverable(
        &self,
    ) -> MutexGuard<'_, Vec<AudioEffect>> {
        lock_recoverable(
            &self.effects,
            "player effects",
            "the effect chain is hot-swappable runtime state",
        )
    }

    /// Recoverable poison policy: effect settings commands are a disposable control queue.
    pub(in crate::playback::player) fn lock_effect_settings_commands_recoverable(
        &self,
    ) -> MutexGuard<'_, Vec<EffectSettingsCommand>> {
        lock_recoverable(
            &self.effect_settings_commands,
            "player effect settings commands",
            "incremental effect settings commands are a disposable control queue",
        )
    }

    /// Recoverable poison policy: pending inline effect updates are a disposable queue.
    pub(in crate::playback::player) fn lock_inline_effects_update_recoverable(
        &self,
    ) -> MutexGuard<'_, Option<InlineEffectsUpdate>> {
        lock_recoverable(
            &self.inline_effects_update,
            "player inline effects update",
            "pending inline effect updates are a disposable queue",
        )
    }

    /// Recoverable poison policy: pending inline track-mix updates are a disposable queue.
    pub(in crate::playback::player) fn lock_inline_track_mix_updates_recoverable(
        &self,
    ) -> MutexGuard<'_, Vec<InlineTrackMixUpdate>> {
        lock_recoverable(
            &self.inline_track_mix_updates,
            "player inline track mix updates",
            "pending inline track-mix updates are a disposable queue",
        )
    }

    /// Recoverable poison policy: DSP metrics are derived telemetry.
    pub(in crate::playback::player) fn lock_dsp_metrics_recoverable(
        &self,
    ) -> MutexGuard<'_, DspChainMetrics> {
        lock_recoverable(
            &self.dsp_metrics,
            "player DSP metrics",
            "DSP metrics are derived telemetry that can be rebuilt",
        )
    }

    /// Recoverable poison policy: the output meter is derived telemetry.
    pub(in crate::playback::player) fn lock_output_meter_recoverable(
        &self,
    ) -> MutexGuard<'_, OutputMeter> {
        lock_recoverable(
            &self.output_meter,
            "player output meter",
            "meter state is derived telemetry that can be rebuilt",
        )
    }

    /// Recoverable poison policy: finished-track bookkeeping can continue from the inner vector.
    pub(in crate::playback::player) fn lock_finished_tracks_recoverable(
        &self,
    ) -> MutexGuard<'_, Vec<i32>> {
        lock_recoverable(
            &self.finished_tracks,
            "player finished tracks",
            "finished-track bookkeeping is rebuildable runtime state",
        )
    }

    /// Recoverable poison policy: pending resume fade is transient runtime configuration.
    pub(in crate::playback::player) fn lock_next_resume_fade_ms_recoverable(
        &self,
    ) -> MutexGuard<'_, Option<f32>> {
        lock_recoverable(
            &self.next_resume_fade_ms,
            "player next resume fade",
            "pending fade configuration is transient runtime state",
        )
    }

    /// Recoverable poison policy: end-of-stream behavior is runtime configuration.
    pub(in crate::playback::player) fn lock_end_of_stream_action_recoverable(
        &self,
    ) -> MutexGuard<'_, EndOfStreamAction> {
        lock_recoverable(
            &self.end_of_stream_action,
            "player end-of-stream action",
            "transport end behavior is runtime configuration",
        )
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::Ordering;

    use crate::container::prot::PathsTrack;
    use crate::playback::player::PlayerState;

    use super::super::Player;

    #[test]
    fn recoverable_effects_lock_returns_inner_after_poison() {
        let player = poison_test_player();
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = player.lock_effects_recoverable();
            panic!("poison player effects");
        }));

        let mut effects = player.lock_effects_recoverable();
        effects.clear();
        assert!(effects.is_empty());
    }

    #[test]
    fn recoverable_sink_lock_returns_inner_after_poison() {
        let player = poison_test_player();
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = player.lock_sink_recoverable();
            panic!("poison player sink");
        }));

        let sink = player.lock_sink_recoverable();
        let _ = sink.len();
    }

    fn poison_test_player() -> Player {
        let player = Player::new_from_file_paths(vec![PathsTrack::new_from_file_paths(vec![
            "/tmp/nonexistent.wav".to_string(),
        ])]);
        player.playback_thread_exists.store(false, Ordering::SeqCst);
        player.abort.store(true, Ordering::SeqCst);
        *player.lock_playback_thread_handle_invariant() = None;
        *player.lock_state_invariant() = PlayerState::Stopped;
        player
    }
}
