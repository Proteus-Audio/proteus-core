use std::{path::Path, fs::File, collections::HashMap};

use symphonia::core::{
    audio::{Channels, Layout}, codecs::CodecParameters, formats::{FormatOptions, Track}, io::{
        MediaSource, MediaSourceStream, ReadOnlySource
    }, meta::MetadataOptions, probe::{
        Hint,
        ProbeResult
    }
};

pub fn get_time_from_frames(codec_params: &CodecParameters) -> f64 {
    let tb = codec_params.time_base.unwrap();
    let dur = codec_params.n_frames.map(|frames| codec_params.start_ts + frames).unwrap();
    let time = tb.calc_time(dur);

    time.seconds as f64 + time.frac
}

pub fn get_probe_result_from_string(file_path: &str) -> ProbeResult {
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

    symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts).unwrap()
}

fn get_durations(file_path: &str) -> HashMap<u32, f64> {
    let mut probed = get_probe_result_from_string(file_path);

    let mut durations: Vec<f64> = Vec::new();

    if let Some(metadata_rev) = probed.format.metadata().current() {
        metadata_rev.tags().iter().for_each(|tag| {
            if tag.key == "DURATION" {
                // Convert duration of type 01:12:37.227000000 to 4337.227
                let duration = tag.value.to_string().clone();
                let duration_parts = duration.split(":").collect::<Vec<&str>>();
                let hours = duration_parts[0].parse::<f64>().unwrap();
                let minutes = duration_parts[1].parse::<f64>().unwrap();
                let seconds = duration_parts[2].parse::<f64>().unwrap();
                // let milliseconds = duration_parts[3].parse::<f64>().unwrap();
                let duration_in_seconds = (hours * 3600.0) + (minutes * 60.0) + seconds;

                durations.push(duration_in_seconds);
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

#[derive(Debug)]
pub struct TrackInfo {
    pub sample_rate: u32,
    pub channel_count: u32,
    pub bits_per_sample: u32,
}

fn get_track_info(track: &Track) -> TrackInfo {
    let codec_params = &track.codec_params;
    let sample_rate = codec_params.sample_rate.unwrap();
    let bits_per_sample = codec_params.bits_per_sample.unwrap();

    let mut channel_count = match codec_params.channel_layout {
        Some(Layout::Mono) => 1,
        Some(Layout::Stereo) => 2,
        Some(Layout::TwoPointOne) => 3,
        Some(Layout::FivePointOne) => 6,
        _ => 0,
    };

    if channel_count == 0 {
        channel_count = codec_params.channels.unwrap_or(Channels::FRONT_CENTRE).iter().count() as u32;
    }
    
    TrackInfo {
        sample_rate,
        channel_count,
        bits_per_sample,
    }
}

fn reduce_track_infos(track_infos: Vec<TrackInfo>) -> TrackInfo {
    let info = track_infos.into_iter().fold(None, |acc: Option<TrackInfo>, track_info| {
        match acc {
            Some(acc) => {
                if acc.sample_rate != track_info.sample_rate {
                    panic!("Sample rates do not match");
                }

                if acc.channel_count != track_info.channel_count {
                    panic!("Channel layouts do not match {} != {}", acc.channel_count, track_info.channel_count);
                }

                if acc.bits_per_sample != track_info.bits_per_sample {
                    panic!("Bits per sample do not match");
                }

                Some(acc)
            },
            None => Some(track_info),
        }
    });

    info.unwrap()
}

fn gather_track_info(file_path: &str) -> TrackInfo {
    let probed = get_probe_result_from_string(file_path);

    let tracks = probed.format.tracks();
    let mut track_infos: Vec<TrackInfo> = Vec::new();
    for track in tracks {
        let track_info = get_track_info(track);
        track_infos.push(track_info);
    }
    
    reduce_track_infos(track_infos)
}

fn gather_track_info_from_file_paths(file_paths: Vec<String>) -> TrackInfo {
    let mut track_infos: Vec<TrackInfo> = Vec::new();

    for file_path in file_paths {
        println!("File path: {:?}", file_path);
        let track_info = gather_track_info(&file_path);
        track_infos.push(track_info);
    }

    reduce_track_infos(track_infos)
}

#[derive(Debug, Clone)]
pub struct Info {
    pub file_paths: Vec<String>,
    pub duration_map: HashMap<u32, f64>,
    pub channels: u32,
    pub sample_rate: u32,
    pub bits_per_sample: u32,
}

impl Info {
    pub fn new(file_path: String) -> Self {
        let track_info = gather_track_info(&file_path);

        Self {
            duration_map: get_durations(&file_path),
            file_paths: vec![file_path],
            channels: track_info.channel_count,
            sample_rate: track_info.sample_rate,
            bits_per_sample: track_info.bits_per_sample,
        }
    }

    pub fn new_from_file_paths(file_paths: Vec<String>) -> Self {
        let mut duration_map: HashMap<u32, f64> = HashMap::new();

        for (index, file_path) in file_paths.iter().enumerate() {
            let durations = get_durations(file_path);
            let longest = durations.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
            duration_map.insert(index as u32, *longest.1);
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
    
    pub fn get_duration(&self, index: u32) -> Option<f64> {
        match self.duration_map.get(&index) {
            Some(duration) => Some(*duration),
            None => None,
        }
    }
}
