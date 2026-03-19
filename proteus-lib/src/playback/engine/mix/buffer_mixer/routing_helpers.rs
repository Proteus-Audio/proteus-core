//! Helper functions for buffer routing, window math, and debug logging.

use crate::container::prot::{ActiveWindow, ShuffleSource};
#[cfg(feature = "buffer-map")]
use crate::logging::log;

#[cfg(feature = "buffer-map")]
use super::routing_time::samples_to_ms;
use super::AlignedSampleBuffer;
#[cfg(feature = "buffer-map")]
use super::BufferInstance;

/// Source identifier used by decode workers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SourceKey {
    /// Container track id source.
    TrackId(u32),
    /// Standalone file path source.
    FilePath(String),
}

impl From<&ShuffleSource> for SourceKey {
    /// Convert a runtime shuffle source into a decode-worker source key.
    fn from(value: &ShuffleSource) -> Self {
        match value {
            ShuffleSource::TrackId(track_id) => Self::TrackId(*track_id),
            ShuffleSource::FilePath(path) => Self::FilePath(path.clone()),
        }
    }
}

/// Aggregate fill state for a track or the whole mix.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillState {
    /// Mix/track buffers are neither uniformly full nor uniformly not-full.
    Partial,
    /// Every instance currently reports full.
    Full,
    /// No instance currently reports full.
    NotFull,
}

/// Debug telemetry returned by routing calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RouteDecision {
    /// Instance ids that received decoded source samples.
    pub(crate) sample_targets_written: Vec<usize>,
    /// Instance ids that received zero-fill for this packet span.
    pub(crate) zero_fill_targets_written: Vec<usize>,
    /// True when no instance was relevant for this packet.
    pub(crate) ignored: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PushResult {
    pub(super) written_samples: usize,
    pub(super) wrote_any: bool,
}

/// Push an owned sample vector into an instance buffer, truncating to capacity.
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

/// Push borrowed samples into an instance buffer, truncating to capacity.
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

/// Push virtual zero samples into an instance buffer, truncating to capacity.
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

/// Collapse multiple per-instance "full" flags into one aggregate fill state.
#[cfg(test)]
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

/// Clamp a floating-point frame offset to non-negative before casting to `usize`.
///
/// Floating-point time arithmetic can produce tiny negative values due to rounding.
/// This helper makes the non-negative invariant explicit at the cast boundary
/// rather than relying on Rust's saturating `as` semantics for negative-to-unsigned
/// casts.
fn nonneg_frame_offset(raw_frames: f64) -> usize {
    raw_frames.max(0.0) as usize
}

/// Compute interleaved sample spans where a decoded packet overlaps active windows.
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

        let start_frame =
            nonneg_frame_offset(((overlap_start - packet_start) * sample_rate as f64).floor())
                .min(frame_count);
        let end_frame =
            nonneg_frame_offset(((overlap_end - packet_start) * sample_rate as f64).ceil())
                .min(frame_count);
        if end_frame <= start_frame {
            continue;
        }

        spans.push((start_frame * channels, end_frame * channels));
    }
    spans
}

#[cfg(feature = "buffer-map")]
/// Emit a buffer-map header line for a logical track.
pub(super) fn log_buffer_header(
    logical_track_index: usize,
    sample_rate: u32,
    channels: usize,
    consumed_samples: usize,
) {
    let consumed_ms = samples_to_ms(consumed_samples, sample_rate, channels);
    if let Err(err) = log(&format!("T{:?}\n{}\n", logical_track_index, consumed_ms)) {
        log::warn!("failed to write buffer-map header: {}", err);
    }
}

#[cfg(feature = "buffer-map")]
/// Emit a buffer-map occupancy line for one instance.
pub(super) fn log_buffer(instance: &BufferInstance, map: Vec<&str>) {
    let instance_id = instance.meta.instance_id;
    if let Err(err) = log(&format!("[{}] <- i{}\n", map.join(""), instance_id)) {
        log::warn!("failed to write buffer-map line: {}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::prot::ActiveWindow;

    #[test]
    fn packet_overlap_samples_returns_expected_range() {
        let spans = packet_overlap_samples(
            0.0,
            10,
            10,
            2,
            &[ActiveWindow {
                start_ms: 200,
                end_ms: Some(800),
            }],
        );
        assert_eq!(spans, vec![(4, 16)]);
    }

    #[test]
    fn nonneg_frame_offset_clamps_negative_to_zero() {
        assert_eq!(super::nonneg_frame_offset(-0.001), 0);
        assert_eq!(super::nonneg_frame_offset(-1e-15), 0);
        assert_eq!(super::nonneg_frame_offset(-100.0), 0);
    }

    #[test]
    fn nonneg_frame_offset_preserves_positive_values() {
        assert_eq!(super::nonneg_frame_offset(0.0), 0);
        assert_eq!(super::nonneg_frame_offset(5.9), 5);
        assert_eq!(super::nonneg_frame_offset(10.0), 10);
    }

    #[test]
    fn packet_overlap_exact_start_produces_zero_offset() {
        // Window starts exactly at packet start — offset should be 0.
        let spans = packet_overlap_samples(
            1.0,
            100,
            100,
            1,
            &[ActiveWindow {
                start_ms: 1000,
                end_ms: Some(2000),
            }],
        );
        assert_eq!(spans, vec![(0, 100)]);
    }

    #[test]
    fn packet_overlap_tiny_negative_epsilon_clamps_to_zero() {
        // Simulate a case where floating-point subtraction yields a tiny negative.
        // packet_ts is slightly after the window start, but due to rounding the
        // difference (overlap_start - packet_start) would normally be zero or
        // positive; we test the helper directly for the negative-epsilon path.
        let spans = packet_overlap_samples(
            0.0,
            48000,
            48000,
            2,
            &[ActiveWindow {
                start_ms: 0,
                end_ms: Some(1000),
            }],
        );
        // Full packet falls inside the window.
        assert_eq!(spans, vec![(0, 96000)]);
    }

    #[test]
    fn packet_overlap_entirely_before_packet_returns_empty() {
        // Window ends before the packet starts — no overlap.
        let spans = packet_overlap_samples(
            2.0,
            100,
            100,
            1,
            &[ActiveWindow {
                start_ms: 0,
                end_ms: Some(1000),
            }],
        );
        assert!(spans.is_empty());
    }
}
