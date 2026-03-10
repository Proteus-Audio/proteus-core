//! Debug-gated diagnostic and fill-state inspection methods for [`BufferMixer`].

#[cfg(any(test, feature = "debug"))]
use super::routing_helpers::{aggregate_fill_state, FillState};
use super::BufferMixer;

impl BufferMixer {
    /// True when each instance in the logical track has samples or is finished.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn track_ready(&self, logical_track_index: usize) -> bool {
        self.track_ready_with_min_samples(logical_track_index, 1)
    }

    /// Fill-state aggregate for all instances.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn instance_buffer_fills(&self) -> Vec<(usize, usize)> {
        self.instances
            .iter()
            .map(|instance| (instance.meta.instance_id, instance.buffer.len()))
            .collect()
    }

    /// Fill-state aggregate for all logical tracks.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn tracks_fill_state(&self) -> Vec<FillState> {
        self.track_instances
            .iter()
            .map(|instance_ids| {
                aggregate_fill_state(
                    instance_ids
                        .iter()
                        .map(|instance_index| self.instances[*instance_index].full),
                )
            })
            .collect()
    }

    /// Fill-state aggregate for one logical track.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn track_fill_state(&self, logical_track_index: usize) -> FillState {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return FillState::NotFull;
        };
        aggregate_fill_state(
            instances
                .iter()
                .map(|instance_index| self.instances[*instance_index].full),
        )
    }

    /// Fill-state aggregate across all logical tracks.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn mix_fill_state(&self) -> FillState {
        aggregate_fill_state(self.instances.iter().map(|instance| instance.full))
    }

    /// Return per-instance debug counters.
    #[cfg(any(test, feature = "debug"))]
    pub(crate) fn counters(&self) -> Vec<(usize, u64, u64, Option<u64>)> {
        self.instances
            .iter()
            .map(|instance| {
                (
                    instance.meta.instance_id,
                    instance.produced_samples,
                    instance.zero_filled_samples,
                    instance.eof_reached_ms,
                )
            })
            .collect()
    }
}
