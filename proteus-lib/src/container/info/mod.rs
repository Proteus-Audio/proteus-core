//! Container metadata helpers and duration probing.

mod aiff;
mod track_info;

use std::{collections::HashMap, fs::File, path::Path};

use log::warn;

use symphonia::core::{
    codecs::CodecParameters,
    errors::Error,
    formats::FormatOptions,
    io::{MediaSource, MediaSourceStream, ReadOnlySource},
    meta::MetadataOptions,
    probe::{Hint, ProbeResult},
    units::TimeBase,
};

use track_info::{gather_track_info, gather_track_info_from_file_paths};

/// Error returned when combining metadata from audio files with incompatible formats.
#[derive(Debug)]
pub enum InfoError {
    /// Track formats differ in a way that prevents mixing (e.g. mismatched sample rates).
    IncompatibleTracks(String),
    /// Symphonia failed to probe or open a media source.
    ProbeFailed(String),
    /// No audio tracks were found in the media source.
    NoTracksFound,
}

impl std::fmt::Display for InfoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompatibleTracks(msg) => write!(f, "incompatible tracks: {}", msg),
            Self::ProbeFailed(msg) => write!(f, "probe failed: {}", msg),
            Self::NoTracksFound => write!(f, "no tracks found in media source"),
        }
    }
}

impl std::error::Error for InfoError {}

/// Convert Symphonia codec parameters to seconds using time base and frames.
pub fn get_time_from_frames(codec_params: &CodecParameters) -> f64 {
    let tb = match codec_params.time_base {
        Some(tb) => tb,
        None => return 0.0,
    };
    let dur = match codec_params.n_frames {
        Some(frames) => codec_params.start_ts + frames,
        None => return 0.0,
    };
    let time = tb.calc_time(dur);

    time.seconds as f64 + time.frac
}

/// Probe a media file (or stdin `-`) and return the Symphonia probe result.
pub fn get_probe_result_from_string(file_path: &str) -> Result<ProbeResult, Error> {
    // If the path string is '-' then read from standard input.
    if file_path == "-" {
        let source = Box::new(ReadOnlySource::new(std::io::stdin())) as Box<dyn MediaSource>;
        return probe_with_hint(source, None);
    }

    let path = Path::new(file_path);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_string());
    let mut hints: Vec<Option<String>> = Vec::new();

    if let Some(ext) = ext.clone() {
        let ext_lc = ext.to_lowercase();
        if ext_lc == "prot" {
            hints.push(Some("mka".to_string()));
        }
        if ext_lc == "aiff" || ext_lc == "aif" || ext_lc == "aifc" || ext_lc == "aaif" {
            hints.push(Some("aiff".to_string()));
            hints.push(Some("aif".to_string()));
            hints.push(Some("aifc".to_string()));
        } else {
            hints.push(Some(ext_lc));
        }
    }

    // Always try without a hint as a fallback.
    hints.push(None);

    for hint in hints {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => return Err(Error::IoError(e)),
        };
        let source = Box::new(file) as Box<dyn MediaSource>;
        if let Ok(probed) = probe_with_hint(source, hint.as_deref()) {
            return Ok(probed);
        }
    }

    Err(Error::IoError(std::io::Error::other(
        "Failed to probe media file",
    )))
}

fn probe_with_hint(
    source: Box<dyn MediaSource>,
    extension_hint: Option<&str>,
) -> Result<ProbeResult, Error> {
    let mut hint = Hint::new();
    if let Some(extension_str) = extension_hint {
        hint.with_extension(extension_str);
    }

    let mss = MediaSourceStream::new(source, Default::default());
    let format_opts = FormatOptions {
        ..Default::default()
    };
    let metadata_opts: MetadataOptions = Default::default();

    symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)
}

/// Best-effort duration mapping per track using metadata or frame counts.
///
/// For container files, this may be approximate if metadata is inaccurate.
pub fn get_durations(file_path: &str) -> HashMap<u32, f64> {
    match try_get_durations(file_path) {
        Ok(durations) => durations,
        Err(err) => {
            warn!(
                "duration probe failed for '{}': {}; using fallback duration mapping",
                file_path, err
            );
            aiff::fallback_durations(file_path)
        }
    }
}

/// Strict duration mapping per track using metadata or frame counts.
///
/// # Errors
///
/// Returns [`InfoError`] when probing fails or no tracks are available.
pub fn try_get_durations(file_path: &str) -> Result<HashMap<u32, f64>, InfoError> {
    let mut probed = get_probe_result_from_string(file_path)
        .map_err(|err| InfoError::ProbeFailed(err.to_string()))?;

    let mut durations: Vec<f64> = Vec::new();

    if let Some(metadata_rev) = probed.format.metadata().current() {
        metadata_rev.tags().iter().for_each(|tag| {
            if tag.key == "DURATION" {
                // Convert duration of type 01:12:37.227000000 to 4337.227
                let duration = tag.value.to_string().clone();
                let duration_parts = duration.split(':').collect::<Vec<&str>>();
                if duration_parts.len() >= 3 {
                    let hours = duration_parts[0].parse::<f64>().unwrap_or(0.0);
                    let minutes = duration_parts[1].parse::<f64>().unwrap_or(0.0);
                    let seconds = duration_parts[2].parse::<f64>().unwrap_or(0.0);
                    let duration_in_seconds = (hours * 3600.0) + (minutes * 60.0) + seconds;
                    durations.push(duration_in_seconds);
                }
            }
        });
    }

    // Convert durations to HashMap with key as index and value as duration
    let mut duration_map: HashMap<u32, f64> = HashMap::new();

    if probed.format.tracks().is_empty() {
        return Err(InfoError::NoTracksFound);
    }

    for (index, track) in probed.format.tracks().iter().enumerate() {
        if let Some(real_duration) = durations.get(index) {
            duration_map.insert(track.id, *real_duration);
            continue;
        }

        let codec_params = &track.codec_params;
        let duration = get_time_from_frames(codec_params);
        duration_map.insert(track.id, duration);
    }

    Ok(duration_map)
}

fn get_durations_best_effort(file_path: &str) -> HashMap<u32, f64> {
    let durations = get_durations(file_path);
    let all_zero = durations.values().all(|value| *value <= 0.0);
    if !durations.is_empty() && !all_zero {
        return durations;
    }

    get_durations_by_scan(file_path)
}

/// Scan all packets to compute per-track durations (accurate but slower).
pub fn get_durations_by_scan(file_path: &str) -> HashMap<u32, f64> {
    match try_get_durations_by_scan(file_path) {
        Ok(durations) => durations,
        Err(err) => {
            warn!(
                "duration scan failed for '{}': {}; using fallback duration mapping",
                file_path, err
            );
            aiff::fallback_durations(file_path)
        }
    }
}

/// Strict packet-scan duration mapping per track.
///
/// # Errors
///
/// Returns [`InfoError`] when probing fails or when no tracks are present.
pub fn try_get_durations_by_scan(file_path: &str) -> Result<HashMap<u32, f64>, InfoError> {
    let mut probed = get_probe_result_from_string(file_path)
        .map_err(|err| InfoError::ProbeFailed(err.to_string()))?;
    if probed.format.tracks().is_empty() {
        return Err(InfoError::NoTracksFound);
    }
    let mut max_ts: HashMap<u32, u64> = HashMap::new();
    let mut time_bases: HashMap<u32, Option<TimeBase>> = HashMap::new();
    let mut sample_rates: HashMap<u32, Option<u32>> = HashMap::new();

    for track in probed.format.tracks().iter() {
        max_ts.insert(track.id, 0);
        time_bases.insert(track.id, track.codec_params.time_base);
        sample_rates.insert(track.id, track.codec_params.sample_rate);
    }

    while let Ok(packet) = probed.format.next_packet() {
        let entry = max_ts.entry(packet.track_id()).or_insert(0);
        if packet.ts() > *entry {
            *entry = packet.ts();
        }
    }

    let mut duration_map: HashMap<u32, f64> = HashMap::new();
    for (track_id, ts) in max_ts {
        let seconds = if let Some(time_base) = time_bases.get(&track_id).copied().flatten() {
            let time = time_base.calc_time(ts);
            time.seconds as f64 + time.frac
        } else if let Some(sample_rate) = sample_rates.get(&track_id).copied().flatten() {
            ts as f64 / sample_rate as f64
        } else {
            0.0
        };
        duration_map.insert(track_id, seconds);
    }

    Ok(duration_map)
}

/// Aggregate codec information for a track.
#[derive(Debug)]
pub struct TrackInfo {
    /// Sample rate of the audio stream, in Hz.
    pub sample_rate: u32,
    /// Number of audio channels in the stream.
    pub channel_count: u32,
    /// Bit depth of the PCM samples, e.g. 16 or 24.
    pub bits_per_sample: u32,
}

/// Combined container info (track list, durations, sample format).
#[derive(Debug, Clone)]
pub struct Info {
    /// Ordered list of source file paths for this container or file set.
    pub file_paths: Vec<String>,
    /// Map from track index to measured duration in seconds.
    pub duration_map: HashMap<u32, f64>,
    /// Number of audio channels shared across all tracks.
    pub channels: u32,
    /// Sample rate shared across all tracks, in Hz.
    pub sample_rate: u32,
    /// Bit depth of the source PCM samples, e.g. 16 or 24.
    pub bits_per_sample: u32,
}

impl Info {
    /// Build info for a single container file.
    ///
    /// Uses metadata-based duration probing first and falls back to a full
    /// packet scan only when metadata is missing or all-zero.
    pub fn new(file_path: String) -> Self {
        let track_info = gather_track_info(&file_path);

        Self {
            duration_map: get_durations_best_effort(&file_path),
            file_paths: vec![file_path],
            channels: track_info.channel_count,
            sample_rate: track_info.sample_rate,
            bits_per_sample: track_info.bits_per_sample,
        }
    }

    /// Build info for a list of standalone files.
    ///
    /// Uses metadata when available and falls back to scanning.
    pub fn new_from_file_paths(file_paths: Vec<String>) -> Self {
        let mut duration_map: HashMap<u32, f64> = HashMap::new();

        for (index, file_path) in file_paths.iter().enumerate() {
            let durations = get_durations_best_effort(file_path);
            let longest = durations
                .iter()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|entry| *entry.1)
                .unwrap_or(0.0);
            duration_map.insert(index as u32, longest);
        }

        let track_info = gather_track_info_from_file_paths(file_paths.clone());

        Self {
            duration_map,
            file_paths,
            channels: track_info.channel_count,
            sample_rate: track_info.sample_rate,
            bits_per_sample: track_info.bits_per_sample,
        }
    }

    /// Get the duration for the given track index, if known.
    pub fn get_duration(&self, index: u32) -> Option<f64> {
        self.duration_map.get(&index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::{codecs::CodecParameters, units::TimeBase};

    #[test]
    fn get_time_from_frames_uses_time_base_when_present() {
        let params = CodecParameters {
            time_base: Some(TimeBase::new(1, 48_000)),
            n_frames: Some(48_000),
            start_ts: 0,
            ..Default::default()
        };
        assert!((get_time_from_frames(&params) - 1.0).abs() < 1e-6);
    }
}
