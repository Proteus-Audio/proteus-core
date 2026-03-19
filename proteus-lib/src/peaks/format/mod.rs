//! Binary peaks file format: reading, writing, and resampling.
//!
//! Responsibilities are split into focused submodules:
//! - `header`: binary header struct, constants, read/write
//! - `io`: `write_peaks_file`, `read_peaks_by_indices`
//! - `query`: sample-range and peak-index math
//! - `resample`: time-alignment and downsampling

mod header;
mod io;
mod query;
mod resample;

use std::fs::File;
use std::io::BufReader;

use super::{GetPeaksOptions, PeaksData, PeaksError};

use header::read_header;
use io::read_peaks_by_indices;
use query::{compute_peak_range, compute_requested_sample_range, should_time_align_peaks};
use resample::{downsample_peaks, time_align_peaks};

pub(super) use io::write_peaks_file;

pub(super) fn read_peaks_with_options(
    path: &str,
    options: &GetPeaksOptions,
) -> Result<PeaksData, PeaksError> {
    if options.target_peaks == Some(0) {
        return Err(PeaksError::InvalidFormat(
            "target_peaks must be greater than zero".to_string(),
        ));
    }

    if options.channels == Some(0) {
        return Err(PeaksError::InvalidFormat(
            "channels must be greater than zero".to_string(),
        ));
    }

    let mut reader = BufReader::new(File::open(path)?);
    let header = read_header(&mut reader)?;
    let (requested_start_sample, requested_end_sample) =
        compute_requested_sample_range(&header, options.start_seconds, options.end_seconds)?;
    let (start_peak, end_peak) =
        compute_peak_range(&header, requested_start_sample, requested_end_sample);
    let mut peaks = read_peaks_by_indices(&mut reader, &header, start_peak, end_peak)?;

    if let Some(requested_channels) = options.channels {
        peaks
            .channels
            .truncate(requested_channels.min(peaks.channels.len()));
    }

    if let Some(target_peaks) = options.target_peaks {
        if should_time_align_peaks(options, header.window_size, target_peaks) {
            peaks = time_align_peaks(
                &peaks,
                &header,
                start_peak,
                requested_start_sample,
                requested_end_sample,
                target_peaks,
            );
        } else {
            downsample_peaks(&mut peaks, target_peaks);
        }
    }

    Ok(peaks)
}

#[cfg(test)]
mod tests;
