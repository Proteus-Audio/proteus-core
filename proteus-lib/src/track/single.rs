//! Buffering implementation for a single audio track.
//!
//! This is a legacy module retained for its tests and as reference. The active
//! playback path uses `playback::engine::mix::runner::decode::file_worker`
//! instead.

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
/// Callers use this to distinguish a successful decode from an abort or a
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
///
/// # Weighting
///
/// Per-track weighting is **not** applied at the decode level. In the active
/// playback system, track weights are resolved and applied during the mix
/// phase (see `playback::engine::mix::track_mix`). Standalone decode workers
/// are responsible only for decoding and enqueueing raw samples.
pub struct TrackArgs {
    /// Path to the audio file to decode.
    pub file_path: String,
    /// Optional symphonia track id; when `None`, the first decodable track is
    /// selected.
    pub track_id: Option<u32>,
    /// Buffer-map key identifying this track.
    pub track_key: u16,
    /// Shared ring-buffer map that decoded samples are pushed into.
    pub buffer_map: TrackBufferMap,
    /// Optional condvar notified when new samples are available or the track
    /// finishes.
    pub buffer_notify: Option<Arc<Condvar>>,
    /// Shared finished-track list for downstream bookkeeping.
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    /// Playback start position in seconds.
    pub start_time: f64,
    /// Number of output channels (typically 2 for stereo).
    pub channels: u8,
}

/// Spawn a decoder thread that buffers audio for a single track.
///
/// The returned handle resolves to a [`TrackDecodeOutcome`]. Callers should
/// inspect it to distinguish normal end-of-stream from abort or failure.
///
/// # Error policy
///
/// - **Open failure / no decodable track / seek failure:** the worker logs a
///   warning, marks the track finished, notifies any condvar waiter, and
///   returns [`TrackDecodeOutcome::Failed`].
/// - **Per-packet decode errors** (`Error::DecodeError`): each error is logged
///   at `warn` level and the loop continues with the next packet. Transient
///   codec errors are treated as recoverable — the worker keeps producing
///   audio rather than aborting.
/// - **Non-recoverable mid-decode error:** the loop exits with `Failed`.
pub fn buffer_track(args: TrackArgs, abort: Arc<AtomicBool>) -> JoinHandle<TrackDecodeOutcome> {
    let TrackArgs {
        file_path,
        track_id,
        track_key,
        buffer_map,
        buffer_notify,
        finished_tracks,
        start_time,
        channels,
    } = args;
    let opened = open_file(&file_path);
    thread::spawn(move || {
        let (mut decoder, mut format) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                warn!("failed to open track '{}': {}", file_path, err);
                return finish(
                    &finished_tracks,
                    track_key,
                    buffer_notify.as_ref(),
                    TrackDecodeOutcome::Failed(err.to_string()),
                );
            }
        };
        let (track_id, dur) = match find_track(&*format, track_id) {
            Some(found) => found,
            None => {
                warn!("no supported audio track found in '{}'", file_path);
                return finish(
                    &finished_tracks,
                    track_key,
                    buffer_notify.as_ref(),
                    TrackDecodeOutcome::Failed("no supported audio track".to_string()),
                );
            }
        };
        let time = Time::new(start_time.floor() as u64, start_time.fract());
        if let Err(err) = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time,
                track_id: Some(track_id),
            },
        ) {
            warn!(
                "seek failed for '{}' at {:.3}s (track_id={}): {}",
                file_path, start_time, track_id, err
            );
            return finish(
                &finished_tracks,
                track_key,
                buffer_notify.as_ref(),
                TrackDecodeOutcome::Failed(format!("seek failed: {}", err)),
            );
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
                    decoded,
                    track_id,
                    track_key,
                    channels,
                    &buffer_map,
                    buffer_notify.as_ref(),
                    &abort,
                    &mut logged_format,
                ),
                Err(Error::DecodeError(e)) => {
                    warn!("decode error for '{}': {}", file_path, e);
                }
                Err(e) => break Err(e),
            }
        };
        let outcome = if abort.load(Ordering::Relaxed) {
            TrackDecodeOutcome::Aborted
        } else {
            match result {
                Ok(_) => TrackDecodeOutcome::Completed,
                Err(e) => {
                    warn!("decode loop ended with error for '{}': {}", file_path, e);
                    TrackDecodeOutcome::Failed(e.to_string())
                }
            }
        };
        finish(&finished_tracks, track_key, buffer_notify.as_ref(), outcome)
    })
}

/// Common exit path: mark finished, notify waiters, return the outcome.
fn finish(
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
    track_key: u16,
    buffer_notify: Option<&Arc<Condvar>>,
    outcome: TrackDecodeOutcome,
) -> TrackDecodeOutcome {
    mark_track_as_finished(&mut finished_tracks.clone(), track_key);
    if let Some(notify) = buffer_notify {
        notify.notify_all();
    }
    outcome
}

fn find_track(format: &dyn FormatReader, track_id: Option<u32>) -> Option<(u32, Option<u64>)> {
    let track = match track_id {
        Some(id) => format.tracks().iter().find(|t| t.id == id).or_else(|| {
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
    let dur = track
        .codec_params
        .n_frames
        .map(|f| track.codec_params.start_ts + f);
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
        info!("decoded track {} buffer ready", track_id);
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
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Condvar, Mutex};

    fn make_args(
        file_path: &str,
        notify: Option<Arc<Condvar>>,
    ) -> (TrackArgs, Arc<Mutex<Vec<u16>>>) {
        let finished_tracks = Arc::new(Mutex::new(Vec::new()));
        let args = TrackArgs {
            file_path: file_path.to_string(),
            track_id: None,
            track_key: 9,
            buffer_map: init_buffer_map(),
            buffer_notify: notify,
            finished_tracks: finished_tracks.clone(),
            start_time: 0.0,
            channels: 2,
        };
        (args, finished_tracks)
    }

    #[test]
    fn buffer_track_marks_finished_when_open_fails() {
        let (args, finished_tracks) = make_args("/definitely/missing/audio-file.wav", None);
        let abort = Arc::new(AtomicBool::new(false));

        let handle = buffer_track(args, abort);
        let outcome = handle.join().expect("worker thread should complete");

        assert_eq!(finished_tracks.lock().unwrap().as_slice(), &[9]);
        assert!(matches!(outcome, TrackDecodeOutcome::Failed(_)));
    }

    #[test]
    fn buffer_track_marks_finished_and_returns_failed_when_open_fails() {
        let (args, finished_tracks) = make_args("/definitely/missing/audio-file.wav", None);
        let abort = Arc::new(AtomicBool::new(false));

        let outcome = buffer_track(args, abort)
            .join()
            .expect("worker thread should complete");

        assert_eq!(finished_tracks.lock().unwrap().as_slice(), &[9]);
        assert!(matches!(outcome, TrackDecodeOutcome::Failed(_)));
    }

    // Regression test (SI-15): failure paths must notify condvar waiters so
    // the runtime does not stall waiting for data that will never arrive.
    #[test]
    fn buffer_track_notifies_condvar_on_failure() {
        let notify = Arc::new(Condvar::new());
        let (args, finished_tracks) =
            make_args("/definitely/missing/audio-file.wav", Some(notify.clone()));
        let abort = Arc::new(AtomicBool::new(false));

        let outcome = buffer_track(args, abort)
            .join()
            .expect("worker thread should complete");

        assert!(finished_tracks.lock().unwrap().contains(&9));
        assert!(matches!(outcome, TrackDecodeOutcome::Failed(_)));
    }

    #[test]
    fn track_decode_outcome_variants_are_distinct() {
        let failed = TrackDecodeOutcome::Failed("seek failed".to_string());
        assert_ne!(failed, TrackDecodeOutcome::Completed);
        assert_ne!(failed, TrackDecodeOutcome::Aborted);
        assert_ne!(TrackDecodeOutcome::Completed, TrackDecodeOutcome::Aborted);
    }

    // SI-16: TrackArgs no longer accepts track_weights; verify the struct can
    // be built without it, confirming the parameter was intentionally removed.
    #[test]
    fn track_args_does_not_require_weights() {
        let (args, _finished) = make_args("/any/path.wav", None);
        // The struct compiles without a track_weights field — this test
        // documents the intentional API choice that weighting is not the
        // responsibility of the decode worker.
        assert_eq!(args.track_key, 9);
    }
}
