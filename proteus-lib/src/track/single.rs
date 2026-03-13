//! Buffering implementation for a single audio track.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::{Decoder, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::units::Time;

use log::{info, warn};

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
    pub buffer_notify: Option<Arc<Condvar>>,
    pub track_weights: Option<Arc<Mutex<HashMap<u16, f32>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
}

/// Spawn a decoder thread that buffers audio for a single track.
pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> JoinHandle<()> {
    let TrackArgs {
        file_path, track_id, track_key, buffer_map, buffer_notify,
        track_weights: _, finished_tracks, start_time, channels,
    } = args;
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
        let (track_id, dur) = match find_track(&*format, track_id) {
            Some(found) => found,
            None => {
                warn!("no supported audio track found in '{}'", file_path);
                mark_track_as_finished(&mut finished_tracks.clone(), track_key);
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
                return;
            }
        };
        let time = Time::new(start_time.floor() as u64, start_time.fract());
        if format
            .seek(SeekMode::Coarse, SeekTo::Time { time, track_id: Some(track_id) })
            .is_err()
        {
            mark_track_as_finished(&mut finished_tracks.clone(), track_key);
            return;
        }
        let mut logged_format = false;
        let result: Result<bool, Error> = loop {
            if abort.load(Ordering::Relaxed) {
                break Ok(true);
            }
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(e) => break Err(e),
            };
            if packet.track_id() != track_id {
                continue;
            }
            if dur.map_or(false, |d| packet.ts() >= d) {
                break Ok(true);
            }
            match decoder.decode(&packet) {
                Ok(decoded) => process_decoded_packet(
                    decoded, track_id, track_key, channels,
                    &buffer_map, buffer_notify.as_ref(), &abort, &mut logged_format,
                ),
                Err(Error::DecodeError(e)) => warn!("decode error: {}", e),
                Err(e) => break Err(e),
            }
        };
        if let Err(e) = result {
            warn!("error: {}", e);
        }
        mark_track_as_finished(&mut finished_tracks.clone(), track_key);
        if let Some(notify) = buffer_notify.as_ref() {
            notify.notify_all();
        }
    })
}

fn find_track(format: &dyn FormatReader, track_id: Option<u32>) -> Option<(u32, Option<u64>)> {
    let track = match track_id {
        Some(id) => format
            .tracks()
            .iter()
            .find(|t| t.id == id)
            .or_else(|| {
                format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            }),
        None => format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL),
    }?;
    let dur = track.codec_params.n_frames.map(|f| track.codec_params.start_ts + f);
    Some((track.id, dur))
}

fn interleave_to_stereo(decoded: AudioBufferRef<'_>, channels: u8) -> Vec<f32> {
    let mut channel_samples: Vec<Vec<f32>> = Vec::new();
    for ch in 0..channels {
        channel_samples.push(process_channel(decoded.clone(), ch as usize));
    }
    let ch1 = channel_samples[0].clone();
    let ch2 = if channel_samples.len() > 1 {
        channel_samples[1].clone()
    } else {
        ch1.clone()
    };
    ch1.into_iter().zip(ch2).flat_map(|(l, r)| [l, r]).collect()
}

fn process_decoded_packet(
    decoded: AudioBufferRef<'_>,
    track_id: u32,
    track_key: u16,
    channels: u8,
    buffer_map: &TrackBufferMap,
    buffer_notify: Option<&Arc<Condvar>>,
    abort: &Arc<AtomicBool>,
    logged_format: &mut bool,
) {
    if !*logged_format {
        info!(
            "Decoded track {} buffer format: {}",
            track_id,
            decoded_format_label(&decoded)
        );
        *logged_format = true;
    }
    let stereo = interleave_to_stereo(decoded, channels);
    if !stereo.is_empty() {
        add_samples_to_buffer_map(
            &mut buffer_map.clone(),
            track_key,
            stereo,
            abort,
            buffer_notify,
        );
    }
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
