//! Time/window math helpers for buffer-mixer routing decisions.

#[cfg(test)]
use crate::container::prot::ActiveWindow;
use crate::dsp::guardrails::{sanitize_channels, sanitize_sample_rate};

use super::BufferInstance;

/// Return true when playback has moved past an instance window and no buffered data remains.
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

/// Return true when a packet timestamp is at or beyond the instance's final window end.
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

/// Convert interleaved sample count to milliseconds for logging and bookkeeping.
pub(super) fn samples_to_ms(samples: usize, sample_rate: u32, channels: usize) -> u64 {
    let frames = samples / sanitize_channels(channels);
    ((frames as f64 / sanitize_sample_rate(sample_rate) as f64) * 1000.0).round() as u64
}

/// Convert milliseconds to an interleaved sample count for the output format.
fn ms_to_samples(ms: u64, sample_rate: u32, channels: usize) -> usize {
    let frames = ((ms as f64 / 1000.0) * sample_rate as f64).round() as usize;
    frames.saturating_mul(channels)
}

/// Convert the final active-window end of an instance into interleaved sample offset.
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

#[cfg(test)]
mod tests {
    use crate::container::prot::{RuntimeInstanceMeta, ShuffleSource};

    use super::*;

    fn instance_with_window(start_ms: u64, end_ms: Option<u64>) -> BufferInstance {
        BufferInstance {
            meta: RuntimeInstanceMeta {
                instance_id: 0,
                logical_track_index: 0,
                slot_index: 0,
                source_key: ShuffleSource::TrackId(1),
                active_windows: vec![ActiveWindow { start_ms, end_ms }],
                selection_index: 0,
                occurrence_index: 0,
            },
            buffer: super::super::AlignedSampleBuffer::with_capacity(16),
            buffer_capacity_samples: 16,
            full: false,
            finished: false,
            produced_samples: 0,
            zero_filled_samples: 0,
            eof_reached_ms: None,
        }
    }

    #[test]
    fn instance_past_window_checks_end_timestamp() {
        let instance = instance_with_window(0, Some(500));
        assert!(!instance_past_window_ts(&instance, &0.4));
        assert!(instance_past_window_ts(&instance, &0.5));
    }

    #[test]
    fn samples_to_ms_converts_interleaved_counts() {
        assert_eq!(samples_to_ms(96_000, 48_000, 2), 1000);
    }
}
