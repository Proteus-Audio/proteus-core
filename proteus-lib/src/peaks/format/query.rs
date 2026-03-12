//! Sample-range and peak-index computation for time-based peak queries.

use super::super::{GetPeaksOptions, PeaksError};
use super::header::Header;

pub(super) fn should_time_align_peaks(
    options: &GetPeaksOptions,
    window_size: u32,
    target_peaks: usize,
) -> bool {
    options.start_seconds.is_some()
        && options.end_seconds.is_some()
        && target_peaks > usize::try_from(window_size.max(1)).unwrap_or(usize::MAX)
}

pub(super) fn compute_requested_sample_range(
    header: &Header,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
) -> Result<(u64, u64), PeaksError> {
    let mut start = start_seconds.unwrap_or(0.0);
    let mut end = end_seconds.unwrap_or(f64::MAX);

    if !start.is_finite() || !end.is_finite() {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be finite numbers".to_string(),
        ));
    }

    if start < 0.0 || end < 0.0 {
        return Err(PeaksError::InvalidFormat(
            "timestamps must be >= 0.0".to_string(),
        ));
    }

    if end < start {
        return Err(PeaksError::InvalidFormat(
            "end_seconds must be >= start_seconds".to_string(),
        ));
    }

    let sample_rate = f64::from(header.sample_rate);

    if end == f64::MAX {
        end = total_samples(header) as f64 / sample_rate;
    }

    // Keep values stable for very large ranges.
    start = start.min(u64::MAX as f64 / sample_rate);
    end = end.min(u64::MAX as f64 / sample_rate);

    let start_sample = (start * sample_rate).floor() as u64;
    let end_sample = (end * sample_rate).ceil() as u64;

    Ok((start_sample, end_sample))
}

pub(super) fn compute_peak_range(header: &Header, start_sample: u64, end_sample: u64) -> (u64, u64) {
    let samples_per_peak = u64::from(header.window_size);
    let start_peak = start_sample / samples_per_peak;
    let mut end_peak = end_sample.div_ceil(samples_per_peak);

    let peak_count = header.peak_count;
    let clamped_start = start_peak.min(peak_count);
    end_peak = end_peak.min(peak_count);
    let clamped_end = end_peak.max(clamped_start);

    (clamped_start, clamped_end)
}

pub(super) fn total_samples(header: &Header) -> u64 {
    header
        .peak_count
        .saturating_mul(u64::from(header.window_size))
}
