//! Schedule-driven source router and logical-track mixer.

mod aligned_buffer;
mod backpressure;
mod diagnostics;
mod mixing;
mod packet_router;
mod routing_helpers;
mod routing_time;

use std::collections::HashSet;
use std::sync::Arc;

use crate::container::prot::{RuntimeInstanceMeta, RuntimeInstancePlan};
#[cfg(feature = "buffer-map")]
use crate::logging::clear_logfile;

use aligned_buffer::AlignedSampleBuffer;
pub(crate) use backpressure::DecodeBackpressure;
use routing_helpers::instance_needs_data;
#[cfg(test)]
pub(crate) use routing_helpers::FillState;
pub(crate) use routing_helpers::{RouteDecision, SourceKey};
use routing_time::instance_fully_past_window;

#[derive(Debug)]
pub(super) struct BufferInstance {
    pub(super) meta: RuntimeInstanceMeta,
    pub(in crate::playback::engine::mix::buffer_mixer) buffer: AlignedSampleBuffer,
    pub(super) buffer_capacity_samples: usize,
    pub(super) full: bool,
    pub(super) finished: bool,
    pub(super) produced_samples: u64,
    pub(super) zero_filled_samples: u64,
    pub(super) eof_reached_ms: Option<u64>,
}

impl BufferInstance {
    /// Create an empty per-instance buffer and counters for one runtime instance.
    fn new(meta: RuntimeInstanceMeta, capacity_samples: usize) -> Self {
        Self {
            meta,
            buffer: AlignedSampleBuffer::with_capacity(capacity_samples.max(1)),
            buffer_capacity_samples: capacity_samples.max(1),
            full: false,
            finished: false,
            produced_samples: 0,
            zero_filled_samples: 0,
            eof_reached_ms: None,
        }
    }
}

/// Router/mixer that owns per-instance buffers and schedule-window alignment.
#[derive(Debug)]
pub(crate) struct BufferMixer {
    pub(super) sample_rate: u32,
    pub(super) channels: usize,
    pub(super) mix_chunk_samples: usize,
    pub(super) consumed_samples: usize,
    pub(super) instances: Vec<BufferInstance>,
    pub(super) track_instances: Vec<Vec<usize>>,
    pub(super) track_mix_settings: Vec<(f32, f32)>,
    slot_to_logical: Vec<Option<usize>>,
    pub(super) decode_backpressure: Arc<DecodeBackpressure>,
    pub(super) crossfade_ms: usize,
    pub(super) pop_warning: Vec<usize>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SectionWriteResult {
    pub(super) wrote_real: bool,
    pub(super) wrote_zero: bool,
}

impl BufferMixer {
    /// Create a new buffer mixer from a runtime instance plan.
    pub(crate) fn new(
        plan: RuntimeInstancePlan,
        sample_rate: u32,
        channels: usize,
        capacity_samples: usize,
        track_mix_settings: Vec<(f32, f32)>,
        mix_chunk_samples: usize,
    ) -> Self {
        #[cfg(feature = "buffer-map")]
        if let Err(err) = clear_logfile() {
            log::warn!("failed to clear buffer-map log file: {}", err);
        }
        let max_slot = plan
            .instances
            .iter()
            .map(|m| m.slot_index)
            .max()
            .unwrap_or(0);
        let mut instances = Vec::with_capacity(plan.instances.len());
        let mut slot_to_logical = vec![None; max_slot + 1];
        for meta in plan.instances {
            if meta.slot_index < slot_to_logical.len() {
                slot_to_logical[meta.slot_index] = Some(meta.logical_track_index);
            }
            instances.push(BufferInstance::new(meta, capacity_samples));
        }

        let mut track_instances = vec![Vec::new(); plan.logical_track_count];
        for (index, instance) in instances.iter().enumerate() {
            if instance.meta.logical_track_index >= track_instances.len() {
                track_instances.resize(instance.meta.logical_track_index + 1, Vec::new());
            }
            track_instances[instance.meta.logical_track_index].push(index);
        }

        let decode_backpressure = Arc::new(DecodeBackpressure::from_instances(&instances));

        Self {
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            mix_chunk_samples: mix_chunk_samples.max(1),
            consumed_samples: 0,
            instances,
            track_instances,
            track_mix_settings,
            slot_to_logical,
            decode_backpressure,
            crossfade_ms: 2,
            pop_warning: Vec::new(),
        }
    }

    /// True when each instance in the logical track has at least `min_samples`
    /// available (or is finished/not currently active).
    pub(crate) fn track_ready_with_min_samples(
        &self,
        logical_track_index: usize,
        min_samples: usize,
    ) -> bool {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return true;
        };

        instances.iter().all(|instance_index| {
            let instance = &self.instances[*instance_index];
            if !instance_needs_data(
                instance,
                self.consumed_samples,
                self.sample_rate,
                self.channels,
            ) {
                return true;
            }
            instance.finished || instance.buffer.len() >= min_samples.max(1)
        })
    }

    /// True when each instance in the logical track is marked finished.
    pub(crate) fn track_finished(&self, logical_track_index: usize) -> bool {
        let Some(instances) = self.track_instances.get(logical_track_index) else {
            return true;
        };
        instances.iter().all(|instance_index| {
            let instance = &self.instances[*instance_index];
            instance.finished
                || instance_fully_past_window(
                    instance,
                    self.consumed_samples,
                    self.sample_rate,
                    self.channels,
                )
        })
    }

    /// True when all logical tracks are ready.
    pub(crate) fn mix_ready(&self) -> bool {
        self.mix_ready_with_min_samples(1)
    }

    /// True when all logical tracks are ready for at least `min_samples`.
    pub(crate) fn mix_ready_with_min_samples(&self, min_samples: usize) -> bool {
        (0..self.track_instances.len())
            .all(|track_index| self.track_ready_with_min_samples(track_index, min_samples))
    }

    /// True when all logical tracks are finished.
    pub(crate) fn mix_finished(&self) -> bool {
        (0..self.track_instances.len()).all(|track_index| self.track_finished(track_index))
    }

    /// Return unique source keys referenced by this runtime plan.
    pub(crate) fn sources(&self) -> Vec<SourceKey> {
        let mut set = HashSet::new();
        for instance in self.instances.iter() {
            set.insert(SourceKey::from(&instance.meta.source_key));
        }
        set.into_iter().collect()
    }

    /// Number of concrete instances in the mixer.
    pub(crate) fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Number of logical tracks represented by the plan.
    pub(crate) fn logical_track_count(&self) -> usize {
        self.track_instances.len()
    }

    /// Count instances that are finished or fully elapsed.
    pub(crate) fn finished_instance_count(&self) -> usize {
        self.instances
            .iter()
            .filter(|instance| {
                instance.finished
                    || instance_fully_past_window(
                        instance,
                        self.consumed_samples,
                        self.sample_rate,
                        self.channels,
                    )
            })
            .count()
    }

    /// Update per-track mix controls using a slot index.
    pub(crate) fn set_track_mix_by_slot(&mut self, slot_index: usize, level: f32, pan: f32) {
        let logical = self
            .slot_to_logical
            .get(slot_index)
            .copied()
            .flatten();
        if let Some(logical_track_index) = logical {
            if logical_track_index < self.track_mix_settings.len() {
                self.track_mix_settings[logical_track_index] =
                    (level.max(0.0), pan.clamp(-1.0, 1.0));
            }
        }
    }

    /// Shared backpressure handle used by decode workers to block until source buffers have room.
    pub(crate) fn decode_backpressure(&self) -> Arc<DecodeBackpressure> {
        Arc::clone(&self.decode_backpressure)
    }
}

#[cfg(test)]
mod tests;
