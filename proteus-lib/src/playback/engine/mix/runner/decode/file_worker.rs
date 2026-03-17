//! Standalone-file decode worker.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use log::{debug, warn};
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::Time;

use crate::tools::decode::open_file;

use super::super::super::buffer_mixer::{DecodeBackpressure, SourceKey};
use super::super::super::decoder_events::DecodeWorkerEvent;
use super::{
    forward_decoded_packet, interleaved_samples, packet_ts_seconds, ForwardInfra, StartupLog,
};

/// Spawn a decode worker for one standalone audio file source.
pub(crate) fn spawn_file_decode_worker(
    file_path: String,
    start_time: f64,
    channels: u8,
    sender: mpsc::SyncSender<DecodeWorkerEvent>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    decode_backpressure: Arc<DecodeBackpressure>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        run_file_decode_worker(
            file_path,
            start_time,
            channels,
            sender,
            abort,
            decode_backpressure,
        )
    })
}

fn run_file_decode_worker(
    file_path: String,
    start_time: f64,
    channels: u8,
    sender: mpsc::SyncSender<DecodeWorkerEvent>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    decode_backpressure: Arc<DecodeBackpressure>,
) {
    let startup_trace = Instant::now();
    let source_key = SourceKey::FilePath(file_path.clone());
    let Some((mut decoder, mut format)) = open_file_decoder(&file_path, &source_key, &sender)
    else {
        return;
    };
    let Some(track) = select_decodable_track(format.as_ref(), &source_key, &sender) else {
        return;
    };
    seek_file_reader(
        format.as_mut(),
        start_time,
        &file_path,
        track.id,
        &source_key,
        &sender,
    );
    let infra = ForwardInfra {
        sender: &sender,
        decode_backpressure: decode_backpressure.as_ref(),
        abort: abort.as_ref(),
        startup_trace,
    };
    decode_file_packets(
        &mut decoder,
        format.as_mut(),
        &track,
        start_time,
        channels,
        &source_key,
        infra,
    );
    let _ = sender.send(DecodeWorkerEvent::SourceFinished { source_key });
}

fn open_file_decoder(
    file_path: &str,
    source_key: &SourceKey,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
) -> Option<crate::tools::decode::OpenedDecoder> {
    match open_file(file_path) {
        Ok(opened) => Some(opened),
        Err(err) => {
            debug!("file worker open failed: source={} err={}", file_path, err);
            let _ = sender.send(DecodeWorkerEvent::SourceError {
                source_key: source_key.clone(),
                recoverable: false,
                message: err.to_string(),
            });
            let _ = sender.send(DecodeWorkerEvent::SourceFinished {
                source_key: source_key.clone(),
            });
            None
        }
    }
}

fn select_decodable_track(
    format: &dyn symphonia::core::formats::FormatReader,
    source_key: &SourceKey,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
) -> Option<symphonia::core::formats::Track> {
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .cloned();
    if track.is_none() {
        let _ = sender.send(DecodeWorkerEvent::SourceError {
            source_key: source_key.clone(),
            recoverable: false,
            message: "no decodable audio track".to_string(),
        });
        let _ = sender.send(DecodeWorkerEvent::SourceFinished {
            source_key: source_key.clone(),
        });
    }
    track
}

fn seek_file_reader(
    format: &mut dyn symphonia::core::formats::FormatReader,
    start_time: f64,
    file_path: &str,
    track_id: u32,
    source_key: &SourceKey,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
) {
    let seconds = start_time.floor() as u64;
    let frac_of_second = start_time.fract();
    let time = Time::new(seconds, frac_of_second);
    if let Err(err) = format.seek(
        SeekMode::Coarse,
        SeekTo::Time {
            time,
            track_id: Some(track_id),
        },
    ) {
        warn!(
            "file decode seek failed, falling back to stream start: source={} err={}",
            file_path, err
        );
        let _ = sender.send(DecodeWorkerEvent::SourceError {
            source_key: source_key.clone(),
            recoverable: true,
            message: format!("seek failed; continuing from stream start: {}", err),
        });
    }
}

fn decode_file_packets(
    decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
    format: &mut dyn symphonia::core::formats::FormatReader,
    track: &symphonia::core::formats::Track,
    start_time: f64,
    channels: u8,
    source_key: &SourceKey,
    infra: ForwardInfra<'_>,
) {
    let mut log = StartupLog {
        logged_first_ready: false,
        logged_first_send: false,
    };
    let time_base = track.codec_params.time_base;
    let sample_rate = track.codec_params.sample_rate;
    loop {
        if infra.abort.load(Ordering::Relaxed) {
            break;
        }

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(ref err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(err) => {
                warn!(
                    "file decode packet-read error: source={:?} err={}",
                    source_key, err
                );
                let _ = infra.sender.send(DecodeWorkerEvent::SourceError {
                    source_key: source_key.clone(),
                    recoverable: false,
                    message: format!("packet-read failed: {}", err),
                });
                break;
            }
        };
        if packet.track_id() != track.id {
            continue;
        }

        let packet_ts = packet_ts_seconds(packet.ts(), time_base, sample_rate, start_time);
        if !decode_and_forward_file_packet(
            decoder, &packet, channels, source_key, &infra, &mut log, packet_ts,
        ) {
            break;
        }
    }
}

fn decode_and_forward_file_packet(
    decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
    packet: &symphonia::core::formats::Packet,
    channels: u8,
    source_key: &SourceKey,
    infra: &ForwardInfra<'_>,
    log: &mut StartupLog,
    packet_ts: f64,
) -> bool {
    match decoder.decode(packet) {
        Ok(decoded) => {
            let samples = interleaved_samples(decoded, channels);
            if samples.is_empty() {
                return true;
            }
            forward_decoded_packet("file", source_key.clone(), packet_ts, samples, infra, log)
        }
        Err(Error::DecodeError(err)) => {
            let _ = infra.sender.send(DecodeWorkerEvent::SourceError {
                source_key: source_key.clone(),
                recoverable: true,
                message: err.to_string(),
            });
            true
        }
        Err(err) => {
            let _ = infra.sender.send(DecodeWorkerEvent::SourceError {
                source_key: source_key.clone(),
                recoverable: false,
                message: err.to_string(),
            });
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use symphonia::core::errors::Error;

    use super::super::super::super::buffer_mixer::SourceKey;
    use super::super::super::super::decoder_events::DecodeWorkerEvent;
    use super::spawn_file_decode_worker;

    #[test]
    fn file_worker_symbol_is_linked() {
        let _ = spawn_file_decode_worker;
    }

    /// Returns true when the error is an EOF (UnexpectedEof), mirroring the
    /// match pattern used in `decode_file_packets`.
    fn is_eof(err: &Error) -> bool {
        matches!(err, Error::IoError(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
    }

    #[test]
    fn eof_error_classified_as_end_of_stream() {
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "end of stream",
        ));
        assert!(is_eof(&err));
    }

    #[test]
    fn io_error_not_classified_as_eof() {
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "disk failure",
        ));
        assert!(!is_eof(&err));
    }

    #[test]
    fn decode_error_not_classified_as_eof() {
        let err = Error::DecodeError("malformed frame");
        assert!(!is_eof(&err));
    }

    #[test]
    fn reset_required_not_classified_as_eof() {
        let err = Error::ResetRequired;
        assert!(!is_eof(&err));
    }

    #[test]
    fn eof_does_not_emit_source_error() {
        let (sender, receiver) = mpsc::sync_channel::<DecodeWorkerEvent>(16);
        let source_key = SourceKey::FilePath("test.wav".to_string());
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "end of stream",
        ));

        // Simulate the EOF branch: no SourceError should be sent
        if !is_eof(&err) {
            let _ = sender.send(DecodeWorkerEvent::SourceError {
                source_key,
                recoverable: false,
                message: err.to_string(),
            });
        }
        drop(sender);

        let events: Vec<_> = receiver.try_iter().collect();
        assert!(
            events.is_empty(),
            "EOF should not produce a SourceError event"
        );
    }

    #[test]
    fn real_io_error_emits_source_error() {
        let (sender, receiver) = mpsc::sync_channel::<DecodeWorkerEvent>(16);
        let source_key = SourceKey::FilePath("test.wav".to_string());
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "disk failure",
        ));

        // Simulate the real-error branch
        if !is_eof(&err) {
            let _ = sender.send(DecodeWorkerEvent::SourceError {
                source_key,
                recoverable: false,
                message: format!("packet-read failed: {}", err),
            });
        }
        drop(sender);

        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 1, "real IO error should emit SourceError");
        assert!(matches!(
            &events[0],
            DecodeWorkerEvent::SourceError {
                recoverable: false,
                ..
            }
        ));
    }
}
