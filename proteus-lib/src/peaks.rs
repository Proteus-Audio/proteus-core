use std::sync::mpsc;
use std::thread;

use log::warn;
use symphonia::core::audio::{AudioBufferRef, Signal, Channels};
use symphonia::core::errors::Error;

use crate::tools::*;

fn convert_signed_24bit_to_f32(sample: i32) -> f32 {
    // Assuming the 24-bit sample is the least significant bits of a 32-bit integer
    // Shift to get rid of padding/sign-extension if necessary
    let shifted_sample = sample << 8 >> 8; // Adjust this based on your data's format

    // Normalize to -1.0 to 1.0 range
    let normalized_sample = shifted_sample as f32 / 2f32.powi(23);

    normalized_sample
}

fn convert_unsigned_24bit_to_f32(sample: u32) -> f32 {
    let shifted_sample = sample as i32 - 2i32.pow(23);
    let normalized_sample = shifted_sample as f32 / 2f32.powi(23);
    normalized_sample
}

fn convert_signed_16bit_to_f32(sample: i16) -> f32 {
    sample as f32 / 2f32.powi(15)
}

fn convert_unsigned_16bit_to_f32(sample: u16) -> f32 {
    let shifted_sample = sample as i16 - 2i16.pow(15);
    let normalized_sample = shifted_sample as f32 / 2f32.powi(15);
    normalized_sample
}

fn find_peaks(samples: &[f32], window_size: usize) -> Vec<(f32, f32)> {
    samples
        .chunks(window_size)
        .map(|window| {
            let max_peak = window.iter().cloned().fold(f32::MIN, f32::max);
            let min_peak = window.iter().cloned().fold(f32::MAX, f32::min);
            (max_peak, min_peak)
        })
        .collect()
}

pub fn get_peaks(file_path: &str, limited: bool) -> Vec<Vec<(f32, f32)>> {
    let (mut decoder, mut format) = open_file(file_path);
    // let (sender, receiver) = mpsc::sync_channel::<Peak>(1);
    let (sender, receiver) = mpsc::sync_channel::<(usize, Vec<f32>)>(1);

    let sample_rate = format
        .tracks()
        .first()
        .unwrap()
        .codec_params
        .sample_rate
        .unwrap();

    let channels = if limited {
        1
    } else {
        let channels_option = &format.tracks().first().unwrap().codec_params.channels.unwrap_or(Channels::FRONT_CENTRE);
        channels_option.iter().count()
    };

    // let channels = match format.tracks().first().unwrap().codec_params.channel_layout.unwrap_or(Layout::Mono) {
    //     Layout::Mono => 1,
    //     Layout::Stereo => 2,
    //     Layout::TwoPointOne => 3,
    //     Layout::FivePointOne => 6,
    // };

    thread::spawn(move || {
        // If not explicitly specified, use the first audio track.
        let track_id = format.tracks().first().unwrap().id;

        // let track_id = track_id.unwrap_or_else(|| {
        //     format.tracks().iter().find(|track| track.codec_params. == symphonia::core::media::Type::Audio)
        //         .expect("no audio track found").id
        // });

        // Get the selected track using the track ID.
        // let track = format.tracks().iter().find(|track| track.id == track_id).expect("no track found");

        // TODO: Use actual time?
        // let actual_time = track.codec_params.time_base.unwrap().calc_time(seek_success.unwrap().actual_ts);

        let _result: Result<bool, Error> = loop {
            // Get the next packet from the format reader.
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(err) => break Err(err),
            };

            if packet.track_id() != track_id {
                continue;
            }

            let process_channel = |decoded, channel| {
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
                        return Vec::new();
                    }
                }
            };

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    for channel in 0..channels {
                        // println!("channel: {}", channel);
                        let samples = process_channel(decoded.clone(), channel);
                        sender.send((channel, samples)).unwrap();
                    }
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
        // mark_track_as_finished(&mut finished_tracks.clone(), track_key);
    });

    let mut samples: Vec<Vec<f32>> = Vec::new();
    // let mut peaks: Vec<(f64, f32)> = Vec::new();

    for peak in receiver {
        // if peak.channel != 0 {
        //     continue;
        // }

        if samples.len() <= peak.0 {
            samples.push(peak.1);
        } else {
            samples[peak.0].extend(peak.1);
        };

        // peaks.push((peak.time, peak.sample));
    }

    let mut peaks = Vec::new();

    for channel in samples {
        let channel_peaks = find_peaks(&channel, sample_rate as usize / 100);
        peaks.push(channel_peaks);
    }

    peaks
}
