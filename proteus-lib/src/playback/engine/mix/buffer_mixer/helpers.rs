//! Helper functions for buffer routing, window math, and debug logging.

use crate::container::prot::ActiveWindow;
#[cfg(feature = "buffer-map")]
use crate::logging::log;

use super::{AlignedSampleBuffer, BufferInstance, FillState};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PushResult {
    pub(super) written_samples: usize,
    pub(super) wrote_any: bool,
}

pub(super) fn push_owned_slice(
    buffer: &mut AlignedSampleBuffer,
    capacity_samples: usize,
    mut slice: Vec<f32>,
    full_flag: &mut bool,
) -> PushResult {
    let mut result = PushResult::default();
    let original_len = slice.len();
    let capacity = capacity_samples.max(1);
    let available = capacity.saturating_sub(buffer.len());
    let to_write = original_len.min(available);
    if to_write > 0 {
        if to_write < original_len {
            slice.truncate(to_write);
        }
        buffer.push_owned_samples(slice);
        result.wrote_any = true;
        result.written_samples = to_write;
    }
    *full_flag = to_write < original_len;
    result
}

pub(super) fn push_slice(
    buffer: &mut AlignedSampleBuffer,
    capacity_samples: usize,
    slice: &[f32],
    full_flag: &mut bool,
) -> PushResult {
    let mut result = PushResult::default();
    let capacity = capacity_samples.max(1);
    let available = capacity.saturating_sub(buffer.len());
    let to_write = slice.len().min(available);
    if to_write > 0 {
        buffer.push_samples_from_slice(&slice[..to_write]);
        result.wrote_any = true;
        result.written_samples = to_write;
    }
    *full_flag = to_write < slice.len();
    result
}

pub(super) fn push_zeros(
    buffer: &mut AlignedSampleBuffer,
    capacity_samples: usize,
    zero_count: usize,
    full_flag: &mut bool,
) -> PushResult {
    let mut result = PushResult::default();
    let capacity = capacity_samples.max(1);
    let available = capacity.saturating_sub(buffer.len());
    let to_write = zero_count.min(available);

    if to_write > 0 {
        buffer.push_zeros(to_write);
        result.written_samples = to_write;
        result.wrote_any = true;
    }

    *full_flag = to_write < zero_count;
    result
}

pub(super) fn aggregate_fill_state<I>(states: I) -> FillState
where
    I: IntoIterator<Item = bool>,
{
    let mut saw = false;
    let mut all_full = true;
    let mut any_full = false;

    for full in states {
        saw = true;
        all_full &= full;
        any_full |= full;
    }

    if !saw || !any_full {
        FillState::NotFull
    } else if all_full {
        FillState::Full
    } else {
        FillState::Partial
    }
}

pub(super) fn packet_overlap_samples(
    packet_ts: f64,
    frame_count: usize,
    sample_rate: u32,
    channels: usize,
    windows: &[ActiveWindow],
) -> Vec<(usize, usize)> {
    let packet_start = packet_ts.max(0.0);
    let packet_end = packet_start + (frame_count as f64 / sample_rate as f64);
    let mut spans = Vec::new();
    for window in windows {
        let window_start = window.start_ms as f64 / 1000.0;
        let window_end = window
            .end_ms
            .map(|end| end as f64 / 1000.0)
            .unwrap_or(f64::INFINITY);

        let overlap_start = packet_start.max(window_start);
        let overlap_end = packet_end.min(window_end);
        if overlap_start >= overlap_end {
            continue;
        }

        let start_frame = (((overlap_start - packet_start) * sample_rate as f64).floor() as usize)
            .min(frame_count);
        let end_frame =
            (((overlap_end - packet_start) * sample_rate as f64).ceil() as usize).min(frame_count);
        if end_frame <= start_frame {
            continue;
        }

        spans.push((start_frame * channels, end_frame * channels));
    }
    spans
}

pub(super) fn instance_needs_data(
    _instance: &BufferInstance,
    _consumed_samples: usize,
    _sample_rate: u32,
    _channels: usize,
) -> bool {
    true
    // Kept intentionally unconditional for strict alignment semantics:
    // all instance buffers advance together via real audio or zero-fill.
}

pub(super) fn instance_fully_past_window(
    instance: &BufferInstance,
    consumed_samples: usize,
    sample_rate: u32,
    channels: usize,
) -> bool {
    let Some(end_sample) = window_end_samples(instance, sample_rate, channels) else {
        return false;
    };
    consumed_samples >= end_sample && instance.buffer.len() == 0
}

pub(super) fn instance_past_window_ts(instance: &BufferInstance, ts: &f64) -> bool {
    let end: Option<f64> = instance
        .meta
        .active_windows
        .last()
        .and_then(|window| window.end_ms.map(|end| end as f64 / 1000.0));
    let Some(end_ts) = end else {
        return false;
    };

    *ts >= end_ts
}

#[cfg(feature = "buffer-map")]
pub(super) fn log_buffer_header(
    logical_track_index: usize,
    sample_rate: u32,
    channels: usize,
    consumed_samples: usize,
) {
    let consumed_ms = samples_to_ms(consumed_samples, sample_rate, channels);
    log(&format!("T{:?}\n{}\n", logical_track_index, consumed_ms));
}

#[cfg(feature = "buffer-map")]
pub(super) fn log_buffer(instance: &BufferInstance, map: Vec<&str>) {
    let instance_id = instance.meta.instance_id;
    log(&format!("[{}] <- i{}\n", map.join(""), instance_id));
}

#[cfg(not(feature = "buffer-map"))]
pub(super) fn log_buffer_header(
    _logical_track_index: usize,
    _sample_rate: u32,
    _channels: usize,
    _consumed_samples: usize,
) {
}

#[cfg(not(feature = "buffer-map"))]
pub(super) fn log_buffer(_instance: &BufferInstance, _map: Vec<&str>) {}

fn window_end_samples(
    instance: &BufferInstance,
    sample_rate: u32,
    channels: usize,
) -> Option<usize> {
    let end_ms = instance
        .meta
        .active_windows
        .last()
        .and_then(|window| window.end_ms);
    end_ms.map(|ms| ms_to_samples(ms, sample_rate, channels))
}

pub(super) fn samples_to_ms(samples: usize, sample_rate: u32, channels: usize) -> u64 {
    let frames = samples / channels.max(1);
    ((frames as f64 / sample_rate.max(1) as f64) * 1000.0).round() as u64
}

fn ms_to_samples(ms: u64, sample_rate: u32, channels: usize) -> usize {
    let frames = ((ms as f64 / 1000.0) * sample_rate as f64).round() as usize;
    frames.saturating_mul(channels)
}
