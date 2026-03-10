//! Packet routing and instance buffer write methods for [`BufferMixer`].

use log::{debug, info, warn};

use crate::dsp::utils::fade_interleaved_per_frame;
use crate::playback::engine::mix::cover_map::{map_cover, Cover, TransitionDirection};

use super::backpressure::DecodeBackpressure;
use super::routing_helpers::{packet_overlap_samples, push_owned_slice, push_slice, push_zeros};
use super::routing_time::{instance_past_window_ts, samples_to_ms};
use super::{BufferInstance, BufferMixer, RouteDecision, SectionWriteResult, SourceKey};

impl BufferMixer {
    /// Route one decoded packet into schedule-owned instance buffers.
    pub(crate) fn route_packet(
        &mut self,
        samples: &[f32],
        source: SourceKey,
        packet_ts: f64,
    ) -> RouteDecision {
        if samples.is_empty() {
            return RouteDecision {
                ignored: true,
                ..RouteDecision::default()
            };
        }

        let frame_count = samples.len() / self.channels;
        if frame_count == 0 {
            return RouteDecision {
                ignored: true,
                ..RouteDecision::default()
            };
        }

        let mut decision = RouteDecision::default();
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if instance.finished {
                continue;
            }

            if SourceKey::from(&instance.meta.source_key) != source {
                continue;
            }

            if instance_past_window_ts(instance, &packet_ts) {
                debug!(
                    "Instance {} (Track {}) is finished!!",
                    instance.meta.instance_id, instance.meta.logical_track_index
                );
                instance.finished = true;
                self.decode_backpressure.on_finished(instance_index);
                continue;
            }

            let overlap = packet_overlap_samples(
                packet_ts,
                frame_count,
                self.sample_rate,
                self.channels,
                &instance.meta.active_windows,
            );

            let cover_transition = self.crossfade_ms * self.sample_rate as usize / 1000;
            let cover = map_cover(&overlap, samples.len(), Some(cover_transition));

            debug!(
                "Instance {} / Track {} / Time {} / Overlap {:?} / Cover {:?}",
                instance.meta.instance_id,
                instance.meta.logical_track_index,
                packet_ts,
                overlap,
                cover,
            );

            let mut wrote_real = false;
            let mut wrote_zero = false;
            for section in cover {
                let result = Self::route_cover_section(
                    section,
                    samples,
                    packet_ts,
                    cover_transition,
                    self.sample_rate,
                    self.channels,
                    self.decode_backpressure.as_ref(),
                    instance_index,
                    instance,
                );
                wrote_real |= result.wrote_real;
                wrote_zero |= result.wrote_zero;
            }

            if wrote_real {
                decision
                    .sample_targets_written
                    .push(instance.meta.instance_id);
            }
            if wrote_zero {
                decision
                    .zero_fill_targets_written
                    .push(instance.meta.instance_id);
            }
        }

        decision.ignored = decision.sample_targets_written.is_empty()
            && decision.zero_fill_targets_written.is_empty();
        decision
    }

    #[allow(clippy::too_many_arguments)]
    fn route_cover_section(
        section: Cover,
        samples: &[f32],
        packet_ts: f64,
        cover_transition: usize,
        sample_rate: u32,
        channels: usize,
        decode_backpressure: &DecodeBackpressure,
        instance_index: usize,
        instance: &mut BufferInstance,
    ) -> SectionWriteResult {
        match section {
            Cover::Overlap((start_sample, end_sample)) => Self::write_overlap(
                samples,
                start_sample,
                end_sample,
                decode_backpressure,
                instance_index,
                instance,
            ),
            Cover::Underlay((start_sample, end_sample)) => Self::write_underlay(
                start_sample,
                end_sample,
                decode_backpressure,
                instance_index,
                instance,
            ),
            Cover::Transition((direction, (start_sample, end_sample))) => Self::write_transition(
                samples,
                packet_ts,
                cover_transition,
                sample_rate,
                channels,
                decode_backpressure,
                direction,
                start_sample,
                end_sample,
                instance_index,
                instance,
            ),
        }
    }

    fn write_overlap(
        samples: &[f32],
        start_sample: usize,
        end_sample: usize,
        decode_backpressure: &DecodeBackpressure,
        instance_index: usize,
        instance: &mut BufferInstance,
    ) -> SectionWriteResult {
        if start_sample >= end_sample || end_sample > samples.len() {
            return SectionWriteResult::default();
        }

        let push = push_slice(
            &mut instance.buffer,
            instance.buffer_capacity_samples,
            &samples[start_sample..end_sample],
            &mut instance.full,
        );
        decode_backpressure.on_samples_pushed(
            instance_index,
            end_sample - start_sample,
            push.written_samples,
            instance.full,
        );
        if push.written_samples < (end_sample - start_sample) {
            warn!(
                "Partial overlap write for i{}: wrote {} / {} samples",
                instance.meta.instance_id,
                push.written_samples,
                end_sample - start_sample
            );
        }
        instance.produced_samples = instance
            .produced_samples
            .saturating_add(push.written_samples as u64);
        SectionWriteResult {
            wrote_real: push.wrote_any,
            wrote_zero: false,
        }
    }

    fn write_underlay(
        start_sample: usize,
        end_sample: usize,
        decode_backpressure: &DecodeBackpressure,
        instance_index: usize,
        instance: &mut BufferInstance,
    ) -> SectionWriteResult {
        let length = end_sample.saturating_sub(start_sample);
        if length == 0 {
            return SectionWriteResult::default();
        }

        let push = push_zeros(
            &mut instance.buffer,
            instance.buffer_capacity_samples,
            length,
            &mut instance.full,
        );
        decode_backpressure.on_samples_pushed(
            instance_index,
            length,
            push.written_samples,
            instance.full,
        );
        if push.written_samples < length {
            warn!(
                "Partial underlay write for i{}: wrote {} / {} samples",
                instance.meta.instance_id, push.written_samples, length
            );
        }
        instance.zero_filled_samples = instance
            .zero_filled_samples
            .saturating_add(push.written_samples as u64);
        SectionWriteResult {
            wrote_real: false,
            wrote_zero: push.wrote_any,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_transition(
        samples: &[f32],
        packet_ts: f64,
        cover_transition: usize,
        sample_rate: u32,
        channels: usize,
        decode_backpressure: &DecodeBackpressure,
        direction: TransitionDirection,
        start_sample: usize,
        end_sample: usize,
        instance_index: usize,
        instance: &mut BufferInstance,
    ) -> SectionWriteResult {
        if start_sample >= end_sample || end_sample > samples.len() {
            return SectionWriteResult::default();
        }

        let slice_length = end_sample - start_sample;
        info!(
            "Transition starting at: {}",
            packet_ts + (samples_to_ms(start_sample, sample_rate, channels) as f64 / 1000.0)
        );

        let (ramp_start, ramp_end) = match direction {
            TransitionDirection::Up => {
                let starting_val =
                    (cover_transition as f32 - slice_length as f32) / cover_transition as f32;
                (starting_val, 1.0)
            }
            TransitionDirection::Down => {
                let ending_val =
                    (cover_transition as f32 - slice_length as f32) / cover_transition as f32;
                (1.0, ending_val)
            }
        };
        info!("Ramp: {:?}", (ramp_start, ramp_end));

        let mut slice = samples[start_sample..end_sample].to_vec();
        fade_interleaved_per_frame(&mut slice, channels, ramp_start, ramp_end);

        let push = push_owned_slice(
            &mut instance.buffer,
            instance.buffer_capacity_samples,
            slice,
            &mut instance.full,
        );
        decode_backpressure.on_samples_pushed(
            instance_index,
            slice_length,
            push.written_samples,
            instance.full,
        );
        if push.written_samples < slice_length {
            warn!(
                "Partial transition write for i{}: wrote {} / {} samples",
                instance.meta.instance_id, push.written_samples, slice_length
            );
        }
        instance.produced_samples = instance
            .produced_samples
            .saturating_add(push.written_samples as u64);
        SectionWriteResult {
            wrote_real: push.wrote_any,
            wrote_zero: false,
        }
    }

    /// Mark all instances for `source_key` as finished.
    pub(crate) fn signal_finish(&mut self, source_key: &SourceKey) {
        let eof_ms = samples_to_ms(self.consumed_samples, self.sample_rate, self.channels);
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if SourceKey::from(&instance.meta.source_key) != *source_key {
                continue;
            }
            if !instance.finished {
                instance.finished = true;
                instance.eof_reached_ms = Some(eof_ms);
                self.decode_backpressure.on_finished(instance_index);
            }
        }
    }

    /// Mark all instances as finished.
    pub(crate) fn signal_finish_all(&mut self) {
        let eof_ms = samples_to_ms(self.consumed_samples, self.sample_rate, self.channels);
        for (instance_index, instance) in self.instances.iter_mut().enumerate() {
            if !instance.finished {
                instance.finished = true;
                instance.eof_reached_ms = Some(eof_ms);
                self.decode_backpressure.on_finished(instance_index);
            }
        }
    }
}
