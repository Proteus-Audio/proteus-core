//! Container metadata helpers and duration probing.

use std::{collections::HashMap, fs::File, path::Path};

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
    // Create a hint to help the format registry guess what format reader is appropriate.
    let mut hint = Hint::new();

    // If the path string is '-' then read from standard input.
    let source = if file_path == "-" {
        Box::new(ReadOnlySource::new(std::io::stdin())) as Box<dyn MediaSource>
    } else {
        // Othwerise, get a Path from the path string.
        let path = Path::new(file_path);

        // Provide the file extension as a hint.
        if let Some(extension) = path.extension() {
            if let Some(extension_str) = extension.to_str() {
                hint.with_extension(extension_str);
            }
        }

        Box::new(File::open(path).expect("failed to open media file")) as Box<dyn MediaSource>
    };

    // Create the media source stream using the boxed media source from above.
    let mss = MediaSourceStream::new(source, Default::default());

    // Use the default options for format readers other than for gapless playback.
    let format_opts = FormatOptions {
        // enable_gapless: !args.is_present("no-gapless"),
        ..Default::default()
    };

    // Use the default options for metadata readers.
    let metadata_opts: MetadataOptions = Default::default();

    // Get the value of the track option, if provided.
    // let track = match args.value_of("track") {
    //     Some(track_str) => track_str.parse::<usize>().ok(),
    //     _ => None,
    // };

    symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)
}

/// Best-effort duration mapping per track using metadata or frame counts.
///
/// For container files, this may be approximate if metadata is inaccurate.
pub fn get_durations(file_path: &str) -> HashMap<u32, f64> {
    let mut probed = match get_probe_result_from_string(file_path) {
        Ok(probed) => probed,
        Err(_) => return HashMap::new(),
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
        Err(_) => return HashMap::new(),
    };
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
        Err(_) => {
            return TrackInfo {
                sample_rate: 0,
                channel_count: 0,
                bits_per_sample: 0,
            }
        }
    };

    let tracks = probed.format.tracks();
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
