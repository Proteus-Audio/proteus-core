//! Buffering implementation for a single audio track.

use log::warn;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::Time;

use crate::audio::buffer::TrackBufferMap;
use crate::tools::tools::open_file;

use super::buffer::{add_samples_to_buffer_map, mark_track_as_finished};
use super::convert::process_channel;

/// Arguments required to buffer a single track into a ring buffer.
pub struct TrackArgs {
    pub file_path: String,
    pub track_id: Option<u32>,
    pub track_key: u16,
    pub buffer_map: TrackBufferMap,
    pub buffer_notify: Option<Arc<std::sync::Condvar>>,
    pub track_weights: Option<Arc<Mutex<HashMap<u16, f32>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
}

/// Spawn a decoder thread that buffers audio for a single track.
pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> Arc<Mutex<bool>> {
    let TrackArgs {
        file_path,
        track_id,
        track_key,
        buffer_map,
        buffer_notify,
        track_weights: _,
        finished_tracks,
        start_time,
        channels,
    } = args;

    let (mut decoder, mut format) = open_file(&file_path);
    let playing: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

    thread::spawn(move || {
        let (track_id, track) = match track_id {
            Some(requested_id) => format
                .tracks()
                .iter()
                .find(|track| track.id == requested_id)
                .map(|track| (requested_id, track))
                .unwrap_or_else(|| {
                    let fallback = format
                        .tracks()
                        .iter()
                        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                        .expect("no track found");
                    (fallback.id, fallback)
                }),
            None => {
                let fallback = format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                    .expect("no track found");
                (fallback.id, fallback)
            }
        };

        let dur = track
            .codec_params
            .n_frames
            .map(|frames| track.codec_params.start_ts + frames);

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

        let _result: Result<bool, Error> = loop {
            if abort.load(std::sync::atomic::Ordering::Relaxed) {
                break Ok(true);
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(err) => break Err(err),
            };

            if packet.track_id() != track_id {
                continue;
            }

            if packet.ts() >= dur.unwrap_or(0) {
                break Ok(true);
            }

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

                    add_samples_to_buffer_map(
                        &mut buffer_map.clone(),
                        track_key,
                        stereo_samples,
                        buffer_notify.as_ref(),
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

        mark_track_as_finished(&mut finished_tracks.clone(), track_key);
        if let Some(notify) = buffer_notify.as_ref() {
            notify.notify_all();
        }
    });

    playing
}
