//! Peak extraction and binary peak-file utilities for waveform display.

mod error;
mod extract;
mod format;

pub use error::PeaksError;

/// A single peak window with maximum and minimum sample amplitude.
#[derive(Debug, Clone, Copy)]
pub struct PeakWindow {
    pub max: f32,
    pub min: f32,
}

/// Peak data for all channels at a fixed window size.
#[derive(Debug, Clone)]
pub struct PeaksData {
    pub sample_rate: u32,
    pub window_size: u32,
    pub channels: Vec<Vec<PeakWindow>>,
}

/// Decode an audio file and write its peaks to a binary file.
///
/// # Arguments
/// * `input_audio_file` - Source audio path.
/// * `output_peaks_file` - Destination binary peaks file path.
///
/// # Errors
/// Returns an error if audio decode fails or if writing the peaks file fails.
pub fn write_peaks(input_audio_file: &str, output_peaks_file: &str) -> Result<(), PeaksError> {
    let peaks = extract::extract_peaks_from_audio(input_audio_file, false)?;
    format::write_peaks_file(output_peaks_file, &peaks)
}

/// Read all peaks from a binary peaks file.
///
/// # Arguments
/// * `peaks_file` - Path to a binary peaks file previously written by [`write_peaks`].
///
/// # Returns
/// Full per-channel peak data.
///
/// # Errors
/// Returns an error if the file cannot be read or has an invalid peaks format.
pub fn get_peaks(peaks_file: &str) -> Result<PeaksData, PeaksError> {
    format::read_peaks_file(peaks_file)
}

/// Read peaks from a binary peaks file for a specific time range in seconds.
///
/// # Arguments
/// * `peaks_file` - Path to a binary peaks file.
/// * `start_seconds` - Start timestamp (inclusive).
/// * `end_seconds` - End timestamp (exclusive).
///
/// # Returns
/// Per-channel peak data for the requested time range.
///
/// # Errors
/// Returns an error if timestamps are invalid, or if file IO/format parsing fails.
pub fn get_peaks_in_range(
    peaks_file: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<PeaksData, PeaksError> {
    format::read_peaks_in_range(peaks_file, start_seconds, end_seconds)
}

/// Decode an audio file directly into in-memory peaks.
///
/// # Arguments
/// * `file_path` - Source audio path.
/// * `limited` - If true, only channel 0 is processed.
///
/// # Returns
/// In-memory per-channel peak data.
///
/// # Errors
/// Returns an error if decoding fails.
pub fn extract_peaks_from_audio(file_path: &str, limited: bool) -> Result<PeaksData, PeaksError> {
    extract::extract_peaks_from_audio(file_path, limited)
}
