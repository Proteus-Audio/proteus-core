use dasp_ring_buffer::Bounded;
use log::warn;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::Time;

use crate::audio::buffer::buffer_remaining_space;
use crate::tools::tools::open_file;

pub struct TrackArgs {
    pub file_path: String,
    pub track_id: Option<u32>,
    pub track_key: u16,
    pub buffer_map: Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
}

pub fn convert_signed_24bit_to_f32(sample: i32) -> f32 {
    // Assuming the 24-bit sample is the least significant bits of a 32-bit integer
    // Shift to get rid of padding/sign-extension if necessary
    let shifted_sample = sample << 8 >> 8; // Adjust this based on your data's format

    // Normalize to -1.0 to 1.0 range
    let normalized_sample = shifted_sample as f32 / 2f32.powi(23);

    normalized_sample
}

pub fn convert_unsigned_24bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i32 - 2i32.pow(23);
    let normalized_sample = shifted_sample as f32 / 2f32.powi(23);
    normalized_sample
}

pub fn convert_signed_16bit_to_f32(sample: i16) -> f32 {
    sample as f32 / 2f32.powi(15)
}

pub fn convert_unsigned_16bit_to_f32(sample: u16) -> f32 {
    let shifted_sample = sample as i16 - 2i16.pow(15);
    let normalized_sample = shifted_sample as f32 / 2f32.powi(15);
    normalized_sample
}

pub fn process_channel(decoded: AudioBufferRef<'_>, channel: usize) -> Vec<f32> {
    match decoded {
        AudioBufferRef::U16(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_unsigned_16bit_to_f32(s))
            .collect(),

        AudioBufferRef::S16(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_signed_16bit_to_f32(s))
            .collect(),

        AudioBufferRef::U24(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_unsigned_24bit_to_f32(s.0))
            .collect(),

        AudioBufferRef::S24(buf) => buf
            .chan(channel)
            .to_vec()
            .into_iter()
            .map(|s| convert_signed_24bit_to_f32(s.0))
            .collect(),

        AudioBufferRef::F32(buf) => buf.chan(0).to_vec().into_iter().collect(),
        _ => {
            // Repeat for the different sample formats.
            unimplemented!();
            // return Vec::new();
        }
    }
}

pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> Arc<Mutex<bool>> {
    let TrackArgs {
        file_path,
        track_id,
        track_key,
        buffer_map,
        finished_tracks,
        start_time,
        channels,
    } = args;
    // Create a channel for sending audio chunks from the decoder to the playback system.
    let (mut decoder, mut format) = open_file(&file_path);
    let playing: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

    thread::spawn(move || {
        // If not explicitly specified, use the first audio track.
        let track_id = track_id.unwrap_or(0);

        // Get the selected track using the track ID.
        let track = format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .expect("no track found");

        // Get the selected track's timebase and duration.
        // let tb = track.codec_params.time_base;
        let dur = track
            .codec_params
            .n_frames
            .map(|frames| track.codec_params.start_ts + frames);

        // Start time at given time
        let seconds = start_time.floor() as u64;
        let frac_of_second = start_time.fract();
        let time = Time::new(seconds, frac_of_second);

        let seek_success = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time,
                track_id: Some(track_id),
            },
        );

        if seek_success.is_err() {
            mark_track_as_finished(&mut finished_tracks.clone(), track_key);
            return;
        }

        // TODO: Use actual time?
        // let actual_time = track.codec_params.time_base.unwrap().calc_time(seek_success.unwrap().actual_ts);

        let _result: Result<bool, Error> = loop {
            if abort.load(std::sync::atomic::Ordering::Relaxed) {
                break Ok(true);
            }

            // Get the next packet from the format reader.
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(err) => break Err(err),
            };

            if packet.track_id() != track_id {
                continue;
            }

            // If playback is finished, break out of the loop.
            if packet.ts() >= dur.unwrap_or(0) {
                break Ok(true);
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut channel_samples = Vec::new();

                    for channel in 0..channels {
                        // println!("channel: {}", channel);
                        let samples = process_channel(decoded.clone(), channel as usize);
                        channel_samples.push(samples);
                    }

                    // TODO: Handle audio channels properly
                    let channel1 = channel_samples[0].clone();
                    let channel2 = if channel_samples.len() > 1 {
                        channel_samples[1].clone()
                    } else {
                        channel_samples[0].clone()
                    };

                    let stereo_samples: Vec<f32> = channel1
                        .into_iter()
                        .zip(channel2.into_iter())
                        .flat_map(|(left, right)| vec![left, right])
                        .collect();

                    if stereo_samples.len() == 0 {
                        continue;
                    }

                    add_samples_to_buffer_map(&mut buffer_map.clone(), track_key, stereo_samples);
                }
                Err(Error::DecodeError(err)) => {
                    // Decode errors are not fatal. Print the error message and try to decode the next
                    // packet as usual.
                    warn!("decode error: {}", err);
                }
                Err(err) => break Err(err),
            }
        };

        // If an error occurred, print the error message.
        if let Err(err) = _result {
            warn!("error: {}", err);
        }

        // Mark the track as finished
        mark_track_as_finished(&mut finished_tracks.clone(), track_key);
    });

    return playing;
}

pub struct ContainerTrackArgs {
    pub file_path: String,
    pub track_entries: Vec<(u16, u32)>,
    pub buffer_map: Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
}

pub fn buffer_container_tracks(args: ContainerTrackArgs, abort: Arc<AtomicBool>) -> Arc<Mutex<bool>> {
    let ContainerTrackArgs {
        file_path,
        track_entries,
        buffer_map,
        finished_tracks,
        start_time,
        channels,
    } = args;

    let (mut format, mut decoders, mut durations, mut key_for_track, valid_entries) = {
        let mut format = crate::tools::tools::get_reader(&file_path);
        let mut decoders: HashMap<u32, Box<dyn Decoder>> = HashMap::new();
        let mut durations: HashMap<u32, Option<u64>> = HashMap::new();
        let mut key_for_track: HashMap<u32, u16> = HashMap::new();
        let mut valid_entries: Vec<(u16, u32)> = Vec::new();

        eprintln!(
            "container tracks requested: {}",
            track_entries
                .iter()
                .map(|(key, id)| format!("{}=>{}", key, id))
                .collect::<Vec<_>>()
                .join(", ")
        );

        for (track_key, track_id) in &track_entries {
            let track = match format.tracks().iter().find(|track| track.id == *track_id) {
                Some(track) => track,
                None => {
                    eprintln!("container track missing: id {}", track_id);
                    continue;
                }
            };

            let dec_opts: DecoderOptions = Default::default();
            let decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &dec_opts)
                .expect("unsupported codec");
            decoders.insert(*track_id, decoder);

            let dur = track
                .codec_params
                .n_frames
                .map(|frames| track.codec_params.start_ts + frames);
            durations.insert(*track_id, dur);
            key_for_track.insert(*track_id, *track_key);
            valid_entries.push((*track_key, *track_id));
        }

        eprintln!(
            "container tracks found: {}",
            valid_entries
                .iter()
                .map(|(key, id)| format!("{}=>{}", key, id))
                .collect::<Vec<_>>()
                .join(", ")
        );

        (format, decoders, durations, key_for_track, valid_entries)
    };

    let playing: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

    thread::spawn(move || {
        if valid_entries.is_empty() {
            warn!("no valid tracks found in container");
            for (track_key, _) in &track_entries {
                mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
            }
            return;
        }

        let seconds = start_time.floor() as u64;
        let frac_of_second = start_time.fract();
        let time = Time::new(seconds, frac_of_second);

        let seek_track_id = valid_entries[0].1;
        let seek_success = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time,
                track_id: Some(seek_track_id),
            },
        );

        if seek_success.is_err() {
            warn!("container seek failed, starting from beginning");
        }

        let mut finished_track_ids: Vec<u32> = Vec::new();

        let _result: Result<bool, Error> = loop {
            if abort.load(std::sync::atomic::Ordering::Relaxed) {
                break Ok(true);
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(err) => break Err(err),
            };

            let track_id = packet.track_id();
            let track_key = match key_for_track.get(&track_id) {
                Some(track_key) => *track_key,
                None => continue,
            };

            if let Some(dur) = durations.get(&track_id).copied().flatten() {
                if packet.ts() >= dur {
                    if !finished_track_ids.contains(&track_id) {
                        finished_track_ids.push(track_id);
                        mark_track_as_finished(&mut finished_tracks.clone(), track_key);
                    }
                    if finished_track_ids.len() == valid_entries.len() {
                        break Ok(true);
                    }
                    continue;
                }
            }

            let decoder = match decoders.get_mut(&track_id) {
                Some(decoder) => decoder,
                None => continue,
            };

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut channel_samples = Vec::new();

                    for channel in 0..channels {
                        let samples = process_channel(decoded.clone(), channel as usize);
                        channel_samples.push(samples);
                    }

                    let channel1 = channel_samples[0].clone();
                    let channel2 = if channel_samples.len() > 1 {
                        channel_samples[1].clone()
                    } else {
                        channel_samples[0].clone()
                    };

                    let stereo_samples: Vec<f32> = channel1
                        .into_iter()
                        .zip(channel2.into_iter())
                        .flat_map(|(left, right)| vec![left, right])
                        .collect();

                    if stereo_samples.is_empty() {
                        continue;
                    }

                    add_samples_to_buffer_map_nonblocking(
                        &mut buffer_map.clone(),
                        track_key,
                        stereo_samples,
                    );
                }
                Err(Error::DecodeError(err)) => {
                    warn!("decode error: {}", err);
                }
                Err(err) => break Err(err),
            }
        };

        if let Err(err) = _result {
            warn!("error: {}", err);
        }

        for (track_key, track_id) in &valid_entries {
            if !finished_track_ids.contains(track_id) {
                mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
            }
        }
    });

    playing
}

fn add_samples_to_buffer_map(
    buffer_map: &mut Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    track_key: u16,
    samples: Vec<f32>,
) {
    while buffer_remaining_space(buffer_map, track_key) < samples.len() {
        thread::sleep(Duration::from_millis(100));
    }

    let mut hash_buffer = buffer_map.lock().unwrap();

    for sample in samples {
        hash_buffer.get_mut(&track_key).unwrap().push(sample);
    }

    drop(hash_buffer);
}

fn add_samples_to_buffer_map_nonblocking(
    buffer_map: &mut Arc<Mutex<HashMap<u16, Bounded<Vec<f32>>>>>,
    track_key: u16,
    samples: Vec<f32>,
) {
    let remaining = buffer_remaining_space(buffer_map, track_key);
    if remaining == 0 {
        return;
    }

    let take = remaining.min(samples.len());
    let mut hash_buffer = buffer_map.lock().unwrap();
    if let Some(buffer) = hash_buffer.get_mut(&track_key) {
        for sample in samples.into_iter().take(take) {
            buffer.push(sample);
        }
    }
    drop(hash_buffer);
}

fn mark_track_as_finished(finished_tracks: &mut Arc<Mutex<Vec<u16>>>, track_key: u16) {
    let mut finished_tracks_copy = finished_tracks.lock().unwrap();
    finished_tracks_copy.push(track_key);
    drop(finished_tracks_copy);
}
