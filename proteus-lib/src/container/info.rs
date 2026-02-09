//! Container metadata helpers and duration probing.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use log::debug;

use symphonia::core::{
    audio::{AudioBufferRef, Channels, Layout},
    codecs::{CodecParameters, DecoderOptions, CODEC_TYPE_NULL},
    errors::Error,
    formats::{FormatOptions, Track},
    io::{MediaSource, MediaSourceStream, ReadOnlySource},
    meta::MetadataOptions,
    probe::{Hint, ProbeResult},
    units::TimeBase,
};
use symphonia::core::sample::SampleFormat;

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
    let ext = path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_string());
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
        let source = Box::new(File::open(path).expect("failed to open media file")) as Box<dyn MediaSource>;
        if let Ok(probed) = probe_with_hint(source, hint.as_deref()) {
            return Ok(probed);
        }
    }

    Err(Error::IoError(std::io::Error::new(
        std::io::ErrorKind::Other,
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
    let mut probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return fallback_durations(file_path),
    };

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
        return fallback_durations(file_path);
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

    duration_map
}

fn get_durations_best_effort(file_path: &str) -> HashMap<u32, f64> {
    let metadata_durations = std::panic::catch_unwind(|| get_durations(file_path)).ok();
    if let Some(durations) = metadata_durations {
        let all_zero = durations.values().all(|value| *value <= 0.0);
        if !durations.is_empty() && !all_zero {
            return durations;
        }
    }

    get_durations_by_scan(file_path)
}

/// Scan all packets to compute per-track durations (accurate but slower).
pub fn get_durations_by_scan(file_path: &str) -> HashMap<u32, f64> {
    let mut probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return fallback_durations(file_path),
    };
    if probed.format.tracks().is_empty() {
        return fallback_durations(file_path);
    }
    let mut max_ts: HashMap<u32, u64> = HashMap::new();
    let mut time_bases: HashMap<u32, Option<TimeBase>> = HashMap::new();
    let mut sample_rates: HashMap<u32, Option<u32>> = HashMap::new();

    for track in probed.format.tracks().iter() {
        max_ts.insert(track.id, 0);
        time_bases.insert(track.id, track.codec_params.time_base);
        sample_rates.insert(track.id, track.codec_params.sample_rate);
    }

    loop {
        match probed.format.next_packet() {
            Ok(packet) => {
                let entry = max_ts.entry(packet.track_id()).or_insert(0);
                if packet.ts() > *entry {
                    *entry = packet.ts();
                }
            }
            Err(_) => break,
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

    duration_map
}

// impl PartialEq for Layout {
//     fn eq(&self, other: &Self) -> bool {
//         // Implement equality comparison logic for Layout
//         match (self, other) {
//             (Layout::Mono, Layout::Mono) => true,
//             (Layout::Stereo, Layout::Stereo) => true,
//             (Layout::TwoPointOne, Layout::TwoPointOne) => true,
//             (Layout::FivePointOne, Layout::FivePointOne) => true,
//             _ => false,
//         }
//     }
// }

/// Aggregate codec information for a track.
#[derive(Debug)]
pub struct TrackInfo {
    pub sample_rate: u32,
    pub channel_count: u32,
    pub bits_per_sample: u32,
}

fn get_track_info(track: &Track) -> TrackInfo {
    let codec_params = &track.codec_params;
    let sample_rate = codec_params.sample_rate.unwrap_or(0);
    let bits_per_sample = codec_params
        .bits_per_sample
        .unwrap_or_else(|| bits_from_sample_format(codec_params.sample_format));

    let mut channel_count = match codec_params.channel_layout {
        Some(Layout::Mono) => 1,
        Some(Layout::Stereo) => 2,
        Some(Layout::TwoPointOne) => 3,
        Some(Layout::FivePointOne) => 6,
        _ => 0,
    };

    if channel_count == 0 {
        channel_count = codec_params
            .channels
            .unwrap_or(Channels::FRONT_CENTRE)
            .iter()
            .count() as u32;
    }

    TrackInfo {
        sample_rate,
        channel_count,
        bits_per_sample,
    }
}

fn bits_from_sample_format(sample_format: Option<SampleFormat>) -> u32 {
    match sample_format {
        Some(SampleFormat::U8 | SampleFormat::S8) => 8,
        Some(SampleFormat::U16 | SampleFormat::S16) => 16,
        Some(SampleFormat::U24 | SampleFormat::S24) => 24,
        Some(SampleFormat::U32 | SampleFormat::S32 | SampleFormat::F32) => 32,
        Some(SampleFormat::F64) => 64,
        None => 0,
    }
}

fn bits_from_decode(file_path: &str) -> u32 {
    let mut probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return 0,
    };

    let (track_id, codec_params) = match probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
    {
        Some(track) => (track.id, track.codec_params.clone()),
        None => return 0,
    };

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = match symphonia::default::get_codecs().make(&codec_params, &dec_opts)
    {
        Ok(decoder) => decoder,
        Err(_) => return 0,
    };

    loop {
        let packet = match probed.format.next_packet() {
            Ok(packet) => packet,
            Err(_) => return 0,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                return match decoded {
                    AudioBufferRef::U8(_) => 8,
                    AudioBufferRef::S8(_) => 8,
                    AudioBufferRef::U16(_) => 16,
                    AudioBufferRef::S16(_) => 16,
                    AudioBufferRef::U24(_) => 24,
                    AudioBufferRef::S24(_) => 24,
                    AudioBufferRef::U32(_) => 32,
                    AudioBufferRef::S32(_) => 32,
                    AudioBufferRef::F32(_) => 32,
                    AudioBufferRef::F64(_) => 64,
                };
            }
            Err(Error::DecodeError(_)) => continue,
            Err(_) => return 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AiffInfo {
    channels: u16,
    sample_rate: f64,
    bits_per_sample: u16,
    sample_frames: u32,
}

fn fallback_track_info(file_path: &str) -> TrackInfo {
    if let Some(info) = parse_aiff_info(file_path) {
        return TrackInfo {
            sample_rate: info.sample_rate.round() as u32,
            channel_count: info.channels as u32,
            bits_per_sample: info.bits_per_sample as u32,
        };
    }

    TrackInfo {
        sample_rate: 0,
        channel_count: 0,
        bits_per_sample: 0,
    }
}

fn fallback_durations(file_path: &str) -> HashMap<u32, f64> {
    if let Some(info) = parse_aiff_info(file_path) {
        let duration = if info.sample_rate > 0.0 {
            info.sample_frames as f64 / info.sample_rate
        } else {
            0.0
        };
        let mut map = HashMap::new();
        map.insert(0, duration);
        return map;
    }

    HashMap::new()
}

fn parse_aiff_info(file_path: &str) -> Option<AiffInfo> {
    let path = Path::new(file_path);
    let ext = path.extension().and_then(|ext| ext.to_str())?.to_lowercase();
    if ext != "aiff" && ext != "aif" && ext != "aifc" && ext != "aaif" {
        return None;
    }

    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"FORM" {
        return None;
    }
    let form_type = &header[8..12];
    if form_type != b"AIFF" && form_type != b"AIFC" {
        return None;
    }

    loop {
        let mut chunk_header = [0u8; 8];
        if file.read_exact(&mut chunk_header).is_err() {
            break;
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_be_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as u64;

        if chunk_id == b"COMM" {
            if chunk_size < 18 {
                return None;
            }
            let mut comm = vec![0u8; chunk_size as usize];
            file.read_exact(&mut comm).ok()?;
            let channels = u16::from_be_bytes([comm[0], comm[1]]);
            let sample_frames = u32::from_be_bytes([comm[2], comm[3], comm[4], comm[5]]);
            let bits_per_sample = u16::from_be_bytes([comm[6], comm[7]]);
            let mut rate_bytes = [0u8; 10];
            rate_bytes.copy_from_slice(&comm[8..18]);
            let sample_rate = extended_80_to_f64(rate_bytes);

            return Some(AiffInfo {
                channels,
                sample_rate,
                bits_per_sample,
                sample_frames,
            });
        }

        let skip = chunk_size + (chunk_size % 2);
        if file.seek(SeekFrom::Current(skip as i64)).is_err() {
            break;
        }
    }

    None
}

fn extended_80_to_f64(bytes: [u8; 10]) -> f64 {
    let sign = (bytes[0] & 0x80) != 0;
    let exponent = (((bytes[0] & 0x7F) as u16) << 8) | bytes[1] as u16;
    let mut mantissa: u64 = 0;
    for i in 0..8 {
        mantissa = (mantissa << 8) | bytes[2 + i] as u64;
    }

    if exponent == 0 && mantissa == 0 {
        return 0.0;
    }
    if exponent == 0x7FFF {
        return f64::NAN;
    }

    let exp = exponent as i32 - 16383;
    let fraction = mantissa as f64 / (1u64 << 63) as f64;
    let value = 2f64.powi(exp) * fraction;
    if sign { -value } else { value }
}

fn reduce_track_infos(track_infos: Vec<TrackInfo>) -> TrackInfo {
    if track_infos.is_empty() {
        return TrackInfo {
            sample_rate: 0,
            channel_count: 0,
            bits_per_sample: 0,
        };
    }

    let info = track_infos
        .into_iter()
        .fold(None, |acc: Option<TrackInfo>, track_info| match acc {
            Some(acc) => {
                if acc.sample_rate != 0
                    && track_info.sample_rate != 0
                    && acc.sample_rate != track_info.sample_rate
                {
                    panic!("Sample rates do not match");
                }

                if acc.channel_count != 0
                    && track_info.channel_count != 0
                    && acc.channel_count != track_info.channel_count
                {
                    panic!(
                        "Channel layouts do not match {} != {}",
                        acc.channel_count, track_info.channel_count
                    );
                }

                if acc.bits_per_sample != 0
                    && track_info.bits_per_sample != 0
                    && acc.bits_per_sample != track_info.bits_per_sample
                {
                    panic!("Bits per sample do not match");
                }

                Some(TrackInfo {
                    sample_rate: if acc.sample_rate == 0 {
                        track_info.sample_rate
                    } else {
                        acc.sample_rate
                    },
                    channel_count: if acc.channel_count == 0 {
                        track_info.channel_count
                    } else {
                        acc.channel_count
                    },
                    bits_per_sample: if acc.bits_per_sample == 0 {
                        track_info.bits_per_sample
                    } else {
                        acc.bits_per_sample
                    },
                })
            }
            None => Some(track_info),
        });

    info.unwrap()
}

fn gather_track_info(file_path: &str) -> TrackInfo {
    let probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return fallback_track_info(file_path),
    };

    let tracks = probed.format.tracks();
    if tracks.is_empty() {
        return fallback_track_info(file_path);
    }
    let mut track_infos: Vec<TrackInfo> = Vec::new();
    for track in tracks {
        let track_info = get_track_info(track);
        track_infos.push(track_info);
    }

    let mut info = reduce_track_infos(track_infos);
    if info.bits_per_sample == 0 {
        let decoded_bits = bits_from_decode(file_path);
        if decoded_bits > 0 {
            info.bits_per_sample = decoded_bits;
        }
    }
    if info.sample_rate == 0 && info.channel_count == 0 && info.bits_per_sample == 0 {
        return fallback_track_info(file_path);
    }
    info
}

fn gather_track_info_from_file_paths(file_paths: Vec<String>) -> TrackInfo {
    let mut track_infos: Vec<TrackInfo> = Vec::new();

    for file_path in file_paths {
        debug!("File path: {:?}", file_path);
        let track_info = gather_track_info(&file_path);
        track_infos.push(track_info);
    }

    reduce_track_infos(track_infos)
}

/// Combined container info (track list, durations, sample format).
#[derive(Debug, Clone)]
pub struct Info {
    pub file_paths: Vec<String>,
    pub duration_map: HashMap<u32, f64>,
    pub channels: u32,
    pub sample_rate: u32,
    pub bits_per_sample: u32,
}

impl Info {
    /// Build info for a single container file by scanning all packets.
    pub fn new(file_path: String) -> Self {
        let track_info = gather_track_info(&file_path);

        Self {
            duration_map: get_durations_by_scan(&file_path),
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
        match self.duration_map.get(&index) {
            Some(duration) => Some(*duration),
            None => None,
        }
    }
}
