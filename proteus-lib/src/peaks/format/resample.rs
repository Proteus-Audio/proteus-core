//! Peak resampling and time-alignment for peak queries.

use super::super::{PeakWindow, PeaksData};
use super::header::Header;
use super::query::total_samples;

/// Parameters shared across all bins in a time-aligned peak pass.
struct AlignContext {
    duration_samples: f64,
    samples_per_peak: f64,
    start_peak: u64,
    end_peak: u64,
    total_samples: f64,
    requested_start_sample: u64,
    target_peaks: usize,
}

pub(super) fn time_align_peaks(
    peaks: &PeaksData,
    header: &Header,
    start_peak: u64,
    requested_start_sample: u64,
    requested_end_sample: u64,
    target_peaks: usize,
) -> PeaksData {
    if target_peaks == 0 {
        return empty_aligned_channels(peaks);
    }

    let ctx = AlignContext {
        duration_samples: requested_end_sample.saturating_sub(requested_start_sample) as f64,
        samples_per_peak: f64::from(header.window_size),
        start_peak,
        end_peak: start_peak
            .saturating_add(peaks.channels.first().map_or(0, |c| c.len()) as u64),
        total_samples: total_samples(header) as f64,
        requested_start_sample,
        target_peaks,
    };

    let channels = peaks
        .channels
        .iter()
        .map(|channel| aligned_channel_peaks(channel, &ctx))
        .collect();

    PeaksData {
        sample_rate: peaks.sample_rate,
        window_size: peaks.window_size,
        channels,
    }
}

pub(super) fn downsample_peaks(peaks: &mut PeaksData, target_peaks: usize) {
    if peaks.channels.is_empty() {
        return;
    }

    let existing_peaks = peaks.channels[0].len();
    if existing_peaks <= target_peaks {
        return;
    }

    for channel in &mut peaks.channels {
        *channel = average_reduce_channel(channel, target_peaks);
    }
}

fn empty_aligned_channels(peaks: &PeaksData) -> PeaksData {
    PeaksData {
        sample_rate: peaks.sample_rate,
        window_size: peaks.window_size,
        channels: peaks.channels.iter().map(|_| Vec::new()).collect(),
    }
}

fn aligned_channel_peaks(channel: &[PeakWindow], ctx: &AlignContext) -> Vec<PeakWindow> {
    (0..ctx.target_peaks)
        .map(|i| aligned_bin_peak(channel, ctx, i))
        .collect()
}

fn aligned_bin_peak(channel: &[PeakWindow], ctx: &AlignContext, bin: usize) -> PeakWindow {
    let bin_start = ctx.requested_start_sample as f64
        + ctx.duration_samples * (bin as f64 / ctx.target_peaks as f64);
    let bin_end = ctx.requested_start_sample as f64
        + ctx.duration_samples * ((bin + 1) as f64 / ctx.target_peaks as f64);
    let bin_width = (bin_end - bin_start).max(0.0);

    if bin_width == 0.0 {
        return PeakWindow { max: 0.0, min: 0.0 };
    }

    let clamped_start = bin_start.clamp(0.0, ctx.total_samples);
    let clamped_end = bin_end.clamp(0.0, ctx.total_samples);
    if clamped_end <= clamped_start {
        return PeakWindow { max: 0.0, min: 0.0 };
    }

    let first_peak = (clamped_start / ctx.samples_per_peak).floor() as u64;
    let last_peak_exclusive = (clamped_end / ctx.samples_per_peak).ceil() as u64;
    let (sum_max, sum_min) = weighted_peak_sum(
        channel,
        ctx,
        first_peak,
        last_peak_exclusive,
        clamped_start,
        clamped_end,
    );

    PeakWindow {
        max: (sum_max / bin_width) as f32,
        min: (sum_min / bin_width) as f32,
    }
}

fn weighted_peak_sum(
    channel: &[PeakWindow],
    ctx: &AlignContext,
    first_peak: u64,
    last_peak_exclusive: u64,
    clamped_start: f64,
    clamped_end: f64,
) -> (f64, f64) {
    let mut sum_max = 0.0_f64;
    let mut sum_min = 0.0_f64;

    for peak_idx in first_peak..last_peak_exclusive {
        if peak_idx < ctx.start_peak || peak_idx >= ctx.end_peak {
            continue;
        }
        let peak_start = peak_idx as f64 * ctx.samples_per_peak;
        let peak_end = peak_start + ctx.samples_per_peak;
        let overlap =
            (clamped_end.min(peak_end) - clamped_start.max(peak_start)).max(0.0);
        if overlap <= 0.0 {
            continue;
        }
        let local_idx = (peak_idx - ctx.start_peak) as usize;
        if let Some(peak) = channel.get(local_idx) {
            sum_max += f64::from(peak.max) * overlap;
            sum_min += f64::from(peak.min) * overlap;
        }
    }

    (sum_max, sum_min)
}

fn average_reduce_channel(channel: &[PeakWindow], target_peaks: usize) -> Vec<PeakWindow> {
    let source_len = channel.len();
    if source_len <= target_peaks {
        return channel.to_vec();
    }

    let mut reduced = Vec::with_capacity(target_peaks);
    for i in 0..target_peaks {
        let start = i * source_len / target_peaks;
        let end = ((i + 1) * source_len / target_peaks).max(start + 1);
        let window = &channel[start..end.min(source_len)];

        let mut sum_max = 0.0_f32;
        let mut sum_min = 0.0_f32;
        for peak in window {
            sum_max += peak.max;
            sum_min += peak.min;
        }
        let count = window.len() as f32;
        reduced.push(PeakWindow {
            max: sum_max / count,
            min: sum_min / count,
        });
    }

    reduced
}
