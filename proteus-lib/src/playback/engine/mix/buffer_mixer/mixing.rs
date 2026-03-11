//! Sample extraction and logical-track mixing for [`BufferMixer`].

use log::{debug, warn};

use super::routing_helpers::instance_needs_data;
#[cfg(feature = "buffer-map")]
use super::routing_helpers::{log_buffer, log_buffer_header};
use super::routing_time::{instance_fully_past_window, samples_to_ms};
use super::BufferMixer;
use crate::playback::engine::mix::track_stage::{
    apply_track_gain_pan, combine_tracks_equal_weight,
};

impl BufferMixer {
    /// Take synchronized mixed samples across all logical tracks.
    pub(crate) fn take_samples(&mut self) -> Option<Vec<f32>> {
        if !self.mix_ready() {
            return None;
        }

        let min_ready_samples = self.min_ready_samples();
        if min_ready_samples == 0 || min_ready_samples == usize::MAX {
            return None;
        }

        let to_consume = min_ready_samples.min(self.mix_chunk_samples);
        if !self.should_consume_samples(to_consume) {
            return None;
        }

        let logical_tracks = self.mix_tracks(to_consume);
        self.consumed_samples = self.consumed_samples.saturating_add(to_consume);
        Some(combine_tracks_equal_weight(&logical_tracks))
    }

    fn min_ready_samples(&mut self) -> usize {
        let track_instances = self.track_instances.clone();
        track_instances
            .iter()
            .map(|track_indices| {
                track_indices
                    .iter()
                    .map(|instance_index| self.instance_available_samples(*instance_index))
                    .min()
                    .unwrap_or(usize::MAX)
            })
            .min()
            .unwrap_or(0)
    }

    fn instance_available_samples(&mut self, instance_index: usize) -> usize {
        let instance = &mut self.instances[instance_index];
        if !instance_needs_data(
            instance,
            self.consumed_samples,
            self.sample_rate,
            self.channels,
        ) {
            return usize::MAX;
        }

        if instance_fully_past_window(
            instance,
            self.consumed_samples,
            self.sample_rate,
            self.channels,
        ) {
            instance.finished = true;
            self.decode_backpressure.on_finished(instance_index);
            return usize::MAX;
        }

        if instance.buffer.len() > 0 {
            instance.buffer.len()
        } else if instance.finished {
            usize::MAX
        } else {
            0
        }
    }

    fn should_consume_samples(&self, to_consume: usize) -> bool {
        self.mix_finished()
            || to_consume >= self.mix_chunk_samples
            || self.decode_backpressure.has_waiters()
    }

    fn mix_tracks(&mut self, to_consume: usize) -> Vec<Vec<f32>> {
        let mut logical_tracks = Vec::with_capacity(self.track_instances.len());
        let track_instances = self.track_instances.clone();

        for (track_index, instance_indices) in track_instances.iter().enumerate() {
            #[cfg(feature = "buffer-map")]
            log_buffer_header(
                track_index,
                self.sample_rate,
                self.channels,
                self.consumed_samples,
            );

            let mut track_buffer = vec![0.0_f32; to_consume];
            for instance_index in instance_indices {
                self.mix_instance_into_track(*instance_index, &mut track_buffer);
            }

            let (level, pan) = self
                .track_mix_settings
                .get(&track_index)
                .copied()
                .unwrap_or((1.0, 0.0));
            apply_track_gain_pan(&mut track_buffer, level, pan, self.channels);
            logical_tracks.push(track_buffer);
        }

        logical_tracks
    }

    fn mix_instance_into_track(&mut self, instance_index: usize, track_buffer: &mut [f32]) {
        let instance = &mut self.instances[instance_index];
        if !instance_needs_data(
            instance,
            self.consumed_samples,
            self.sample_rate,
            self.channels,
        ) || instance_fully_past_window(
            instance,
            self.consumed_samples,
            self.sample_rate,
            self.channels,
        ) {
            return;
        }

        #[cfg(feature = "buffer-map")]
        let mut logging = BufferLog::new(track_buffer.len());
        let mut popped_samples = 0usize;
        for sample in track_buffer.iter_mut() {
            if let Some(value) = instance.buffer.pop_front() {
                popped_samples = popped_samples.saturating_add(1);
                #[cfg(feature = "buffer-map")]
                logging.observe(value);
                *sample += value;
            }
        }

        self.decode_backpressure
            .on_samples_popped(instance_index, popped_samples);
        debug!("Popped {} samples from i{}", popped_samples, instance_index);

        if popped_samples == 0 && !self.pop_warning.contains(&instance.meta.instance_id) {
            warn!(
                "ZERO! i{} ( finished: {}, ts: {}, total_samples: {} )",
                instance.meta.instance_id,
                instance.finished,
                samples_to_ms(self.consumed_samples, self.sample_rate, self.channels),
                instance.produced_samples + instance.zero_filled_samples
            );
            self.pop_warning.push(instance.meta.instance_id);
        }

        #[cfg(feature = "buffer-map")]
        log_buffer(instance, logging.finish());
    }
}

#[cfg(feature = "buffer-map")]
struct BufferLog {
    divisor: usize,
    count: usize,
    aggregate: f32,
    entries: Vec<&'static str>,
}

#[cfg(feature = "buffer-map")]
impl BufferLog {
    fn new(sample_count: usize) -> Self {
        let divisor = 176;
        Self {
            divisor,
            count: 1,
            aggregate: 0.0,
            entries: Vec::with_capacity((sample_count as f64 / divisor as f64).ceil() as usize),
        }
    }

    fn observe(&mut self, value: f32) {
        self.count += 1;
        if self.count % self.divisor == 0 {
            self.entries
                .push(if self.aggregate != 0.0 { "X" } else { "_" });
        }
        self.aggregate += value;
    }

    fn finish(self) -> Vec<&'static str> {
        self.entries
    }
}
