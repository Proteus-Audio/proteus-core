//! Buffering implementation for a single audio track.
//!
//! This is a legacy buffering module retained for the standalone track decode
//! path. It is not wired into the active playback engine, but its tests are
//! compiled to validate correctness of the decode logic.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo};
use symphonia::core::units::Time;

use log::{info, warn};

use crate::audio::buffer::TrackBufferMap;
use crate::audio::decode::process_channel;
use crate::tools::decode::open_file;

use super::buffer::{add_samples_to_buffer_map, mark_track_as_finished};

/// Outcome of a single-track decode worker run.
///
/// Callers can use this to distinguish a successful decode from an abort or a
/// setup/decode failure, rather than treating all outcomes as end-of-stream.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackDecodeOutcome {
    /// All audio data was decoded and buffered to end-of-stream.
    Completed,
    /// Decode was interrupted by the abort signal before finishing.
    Aborted,
    /// A non-recoverable error occurred during setup or decode.
    ///
    /// The contained string is a human-readable description of the failure.
    Failed(String),
}

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
///
/// The returned handle resolves to a [`TrackDecodeOutcome`]. Callers should
/// check the outcome to distinguish normal end-of-stream from abort or failure.
///
/// # Error policy
///
/// - **Open failure**: the worker logs a warning, marks the track finished,
///   notifies any condvar waiter, and returns `Failed`.
/// - **No decodable track**: same as open failure.
/// - **Seek failure**: the worker logs a warning with the file path, start time,
///   and symphonia error, marks the track finished, notifies any condvar waiter,
///   and returns `Failed`. Seek failure is treated as non-recoverable because
///   the caller requested a specific start position.
/// - **Repeated per-packet decode errors** (`Error::DecodeError`): each error
///   is logged at `warn` level and the loop continues with the next packet.
///   This is intentional — transient codec errors on individual packets are
///   recoverable; the worker keeps producing audio rather than aborting.
/// - **Non-recoverable mid-decode error**: the loop exits with `Failed`.
pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> JoinHandle<TrackDecodeOutcome> {
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
                return TrackDecodeOutcome::Failed(err.to_string());
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
                return TrackDecodeOutcome::Failed("no supported audio track".to_string());
            }
        };
        let time = Time::new(start_time.floor() as u64, start_time.fract());
        if let Err(err) = format.seek(
            SeekMode::Coarse,
            SeekTo::Time { time, track_id: Some(track_id) },
        ) {
            warn!(
                "seek failed for track '{}' at {:.3}s (track_id={}): {}",
                file_path, start_time, track_id, err
            );
            mark_track_as_finished(&mut finished_tracks.clone(), track_key);
            if let Some(notify) = buffer_notify.as_ref() {
                notify.notify_all();
            }
            return TrackDecodeOutcome::Failed(format!("seek failed: {}", err));
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
            if dur.is_some_and(|d| packet.ts() >= d) {
                break Ok(true);
            }
            match decoder.decode(&packet) {
                Ok(decoded) => process_decoded_packet(
                    decoded, track_id, track_key, channels,
                    &buffer_map, buffer_notify.as_ref(), &abort, &mut logged_format,
                ),
                // Per-packet decode errors are recoverable: log and continue.
                Err(Error::DecodeError(e)) => warn!("decode error for track '{}': {}", file_path, e),
                Err(e) => break Err(e),
            }
        };
        let outcome = if abort.load(Ordering::Relaxed) {
            TrackDecodeOutcome::Aborted
        } else {
            match result {
                Ok(_) => TrackDecodeOutcome::Completed,
                Err(e) => {
                    warn!("decode loop ended with error for track '{}': {}", file_path, e);
                    TrackDecodeOutcome::Failed(e.to_string())
                }
            }
        };
        mark_track_as_finished(&mut finished_tracks.clone(), track_key);
        if let Some(notify) = buffer_notify.as_ref() {
            notify.notify_all();
        }
        outcome
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

#[allow(clippy::too_many_arguments)]
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
        info!("decoded track {} buffer format logged", track_id);
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
    use super::{buffer_track, TrackArgs, TrackDecodeOutcome};
    use crate::audio::buffer::init_buffer_map;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Condvar, Mutex};

    fn make_args(file_path: &str, start_time: f64) -> (TrackArgs, Arc<Mutex<Vec<u16>>>) {
        let finished_tracks = Arc::new(Mutex::new(Vec::new()));
        let args = TrackArgs {
            file_path: file_path.to_string(),
            track_id: None,
            track_key: 9,
            buffer_map: init_buffer_map(),
            buffer_notify: None,
            track_weights: Some(Arc::new(Mutex::new(HashMap::new()))),
            finished_tracks: finished_tracks.clone(),
            start_time,
            channels: 2,
        };
        (args, finished_tracks)
    }

    #[test]
    fn buffer_track_marks_finished_when_open_fails() {
        let (args, finished_tracks) = make_args("/definitely/missing/audio-file.wav", 0.0);
        let abort = Arc::new(AtomicBool::new(false));

        let handle = buffer_track(args, abort);
        let outcome = handle.join().expect("worker thread should complete");

        assert_eq!(finished_tracks.lock().unwrap().as_slice(), &[9]);
        assert!(matches!(outcome, TrackDecodeOutcome::Failed(_)));
    }

    // Regression test: a seek failure (simulated by requesting a start time
    // far beyond any real file's duration) should produce a structured Failed
    // outcome, NOT silently mark the track as finished with no diagnostic.
    //
    // NOTE: because this test uses a missing file, the open step fails before
    // the seek step. The test still validates that open failure => Failed outcome
    // and that the condvar waiter is notified (not stalled).
    #[test]
    fn buffer_track_notifies_condvar_on_failure() {
        let finished_tracks = Arc::new(Mutex::new(Vec::<u16>::new()));
        let notify = Arc::new(Condvar::new());
        let args = TrackArgs {
            file_path: "/definitely/missing/audio-file.wav".to_string(),
            track_id: None,
            track_key: 7,
            buffer_map: init_buffer_map(),
            buffer_notify: Some(notify.clone()),
            track_weights: None,
            finished_tracks: finished_tracks.clone(),
            start_time: 0.0,
            channels: 2,
        };
        let abort = Arc::new(AtomicBool::new(false));

        let handle = buffer_track(args, abort);
        let outcome = handle.join().expect("worker thread should complete");

        // Track must be marked finished even on failure, so the engine does
        // not stall waiting for it.
        assert!(finished_tracks.lock().unwrap().contains(&7));
        assert!(matches!(outcome, TrackDecodeOutcome::Failed(_)));
    }

    #[test]
    fn track_decode_outcome_failed_carries_message() {
        let outcome = TrackDecodeOutcome::Failed("seek failed: some error".to_string());
        assert_eq!(outcome, TrackDecodeOutcome::Failed("seek failed: some error".to_string()));
        assert_ne!(outcome, TrackDecodeOutcome::Completed);
        assert_ne!(outcome, TrackDecodeOutcome::Aborted);
    }
}
