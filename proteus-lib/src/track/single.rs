//! Buffering implementation for a single audio track.

use log::{info, warn};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::Time;

use crate::audio::buffer::TrackBufferMap;
use crate::audio::decode::{decoded_format_label, process_channel};
use crate::tools::decode::open_file;

use super::buffer::{add_samples_to_buffer_map, mark_track_as_finished};

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
pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> JoinHandle<()> {
    let TrackArgs {
        file_path,
        track_id,
        track_key,
        buffer_map,
        buffer_notify,
        track_weights,
        finished_tracks,
        start_time,
        channels,
    } = args;
    let _track_weights = track_weights;
    // TODO: Apply `_track_weights` to scale per-track samples when weighting single-track buffers.

    let opened = open_file(&file_path);

    thread::spawn(move || {
        let (mut decoder, mut format) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                warn!("failed to open track '{}': {}", file_path, err);
                mark_track_as_finished(&mut finished_tracks.clone(), track_key);
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
                return;
            }
        };

        // Locate the track to decode. Use the requested track ID if present,
        // otherwise fall back to the first supported audio track.
        let found = match track_id {
            Some(requested_id) => format
                .tracks()
                .iter()
                .find(|t| t.id == requested_id)
                .or_else(|| {
                    format
                        .tracks()
                        .iter()
                        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                }),
            None => format
                .tracks()
                .iter()
                .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL),
        };

        // Extract the values we need from the track reference before using
        // `format` mutably for seek/decode.
        let (track_id, dur) = match found {
            Some(track) => {
                let dur = track
                    .codec_params
                    .n_frames
                    .map(|frames| track.codec_params.start_ts + frames);
                (track.id, dur)
            }
            None => {
                warn!("no supported audio track found in '{}'", file_path);
                mark_track_as_finished(&mut finished_tracks.clone(), track_key);
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
                return;
            }
        };

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

        let mut logged_format = false;

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

            if let Some(dur) = dur {
                if packet.ts() >= dur {
                    break Ok(true);
                }
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if !logged_format {
                        info!(
                            "Decoded track {} buffer format: {}",
                            track_id,
                            decoded_format_label(&decoded)
                        );
                        logged_format = true;
                    }
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
                        &abort,
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
    })
}

#[cfg(test)]
mod tests {
    use super::{buffer_track, TrackArgs};
    use crate::audio::buffer::init_buffer_map;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    #[test]
    fn buffer_track_marks_finished_when_open_fails() {
        let finished_tracks = Arc::new(Mutex::new(Vec::new()));
        let args = TrackArgs {
            file_path: "/definitely/missing/audio-file.wav".to_string(),
            track_id: None,
            track_key: 9,
            buffer_map: init_buffer_map(),
            buffer_notify: None,
            track_weights: Some(Arc::new(Mutex::new(HashMap::new()))),
            finished_tracks: finished_tracks.clone(),
            start_time: 0.0,
            channels: 2,
        };
        let abort = Arc::new(AtomicBool::new(false));

        let handle = buffer_track(args, abort);
        handle.join().expect("worker thread should complete");

        assert_eq!(finished_tracks.lock().unwrap().as_slice(), &[9]);
    }
}
