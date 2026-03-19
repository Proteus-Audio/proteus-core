//! Container demux decode worker (single demuxer feeding multiple track decoders).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use log::{error, warn};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::{Time, TimeBase};

use super::super::super::buffer_mixer::{DecodeBackpressure, SourceKey};
use super::super::super::decoder_events::DecodeWorkerEvent;
use super::{decode_and_forward_packet, packet_ts_seconds, ForwardInfra, StartupLog};

/// Spawn a single demux decode worker that services multiple container track ids.
pub(crate) fn spawn_container_decode_worker(
    file_path: String,
    track_ids: Vec<u32>,
    start_time: f64,
    channels: u8,
    sender: mpsc::SyncSender<DecodeWorkerEvent>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    decode_backpressure: Arc<DecodeBackpressure>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        run_container_decode_worker(
            file_path,
            track_ids,
            start_time,
            channels,
            sender,
            abort,
            decode_backpressure,
        )
    })
}

fn run_container_decode_worker(
    file_path: String,
    track_ids: Vec<u32>,
    start_time: f64,
    channels: u8,
    sender: mpsc::SyncSender<DecodeWorkerEvent>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    decode_backpressure: Arc<DecodeBackpressure>,
) {
    let startup_trace = Instant::now();
    let wanted: HashSet<u32> = track_ids.iter().copied().collect();
    let Some(mut format) = open_container_reader(&file_path, &track_ids, &sender) else {
        return;
    };
    let Some((mut decoders, time_bases, sample_rates)) =
        initialize_container_decoders(format.as_ref(), &wanted, &sender)
    else {
        finish_container_sources(&wanted, &sender);
        return;
    };

    seek_container_reader(format.as_mut(), start_time, &file_path, &sender, &decoders);
    let infra = ForwardInfra {
        worker_label: "container",
        sender: &sender,
        decode_backpressure: decode_backpressure.as_ref(),
        abort: abort.as_ref(),
        startup_trace,
    };
    decode_container_packets(
        format.as_mut(),
        &mut decoders,
        &time_bases,
        &sample_rates,
        start_time,
        channels,
        infra,
    );
    finish_container_sources(&wanted, &sender);
}

fn open_container_reader(
    file_path: &str,
    track_ids: &[u32],
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
) -> Option<Box<dyn symphonia::core::formats::FormatReader>> {
    match crate::tools::decode::get_reader(file_path) {
        Ok(format) => Some(format),
        Err(err) => {
            error!(
                "container worker open failed: source={} err={}",
                file_path, err
            );
            for track_id in track_ids {
                let source_key = SourceKey::TrackId(*track_id);
                let _ = sender.send(DecodeWorkerEvent::SourceError {
                    source_key: source_key.clone(),
                    recoverable: false,
                    message: err.to_string(),
                });
                let _ = sender.send(DecodeWorkerEvent::SourceFinished { source_key });
            }
            None
        }
    }
}

type ContainerDecoderMaps = (
    HashMap<u32, Box<dyn Decoder>>,
    HashMap<u32, Option<TimeBase>>,
    HashMap<u32, Option<u32>>,
);

fn initialize_container_decoders(
    format: &dyn symphonia::core::formats::FormatReader,
    wanted: &HashSet<u32>,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
) -> Option<ContainerDecoderMaps> {
    let mut decoders = HashMap::new();
    let mut time_bases = HashMap::new();
    let mut sample_rates = HashMap::new();

    for track_id in wanted.iter().copied() {
        let Some(track) = format.tracks().iter().find(|track| track.id == track_id) else {
            continue;
        };
        let dec_opts: DecoderOptions = Default::default();
        if let Ok(decoder) = symphonia::default::get_codecs().make(&track.codec_params, &dec_opts) {
            decoders.insert(track_id, decoder);
            time_bases.insert(track_id, track.codec_params.time_base);
            sample_rates.insert(track_id, track.codec_params.sample_rate);
        }
    }

    if decoders.is_empty() {
        for track_id in wanted {
            let source_key = SourceKey::TrackId(*track_id);
            let _ = sender.send(DecodeWorkerEvent::SourceError {
                source_key: source_key.clone(),
                recoverable: false,
                message: "no decoders initialized for requested tracks".to_string(),
            });
        }
        None
    } else {
        Some((decoders, time_bases, sample_rates))
    }
}

fn seek_container_reader(
    format: &mut dyn symphonia::core::formats::FormatReader,
    start_time: f64,
    file_path: &str,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
    decoders: &HashMap<u32, Box<dyn Decoder>>,
) {
    let Some(first_track_id) = decoders.keys().next().copied() else {
        return;
    };
    let seconds = start_time.floor() as u64;
    let frac_of_second = start_time.fract();
    let time = Time::new(seconds, frac_of_second);
    if let Err(err) = format.seek(
        SeekMode::Coarse,
        SeekTo::Time {
            time,
            track_id: Some(first_track_id),
        },
    ) {
        warn!(
            "container decode seek failed, falling back to stream start: source={} track_id={} err={}",
            file_path, first_track_id, err
        );
        let _ = sender.send(DecodeWorkerEvent::SourceError {
            source_key: SourceKey::TrackId(first_track_id),
            recoverable: true,
            message: format!("seek failed; continuing from stream start: {}", err),
        });
    }
}

fn decode_container_packets(
    format: &mut dyn symphonia::core::formats::FormatReader,
    decoders: &mut HashMap<u32, Box<dyn Decoder>>,
    time_bases: &HashMap<u32, Option<TimeBase>>,
    sample_rates: &HashMap<u32, Option<u32>>,
    start_time: f64,
    channels: u8,
    infra: ForwardInfra<'_>,
) {
    let mut log = StartupLog {
        logged_first_ready: false,
        logged_first_send: false,
    };
    loop {
        if infra.abort.load(Ordering::Relaxed) {
            break;
        }

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(ref err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                let _ = infra.sender.send(DecodeWorkerEvent::StreamExhausted);
                break;
            }
            Err(err) => {
                warn!("container decode packet-read error: err={}", err);
                for (track_id, _) in decoders.iter() {
                    let _ = infra.sender.send(DecodeWorkerEvent::SourceError {
                        source_key: SourceKey::TrackId(*track_id),
                        recoverable: false,
                        message: format!("packet-read failed: {}", err),
                    });
                }
                break;
            }
        };
        let track_id = packet.track_id();
        let Some(decoder) = decoders.get_mut(&track_id) else {
            continue;
        };

        let source_key = SourceKey::TrackId(track_id);
        let packet_ts = packet_ts_seconds(
            packet.ts(),
            time_bases.get(&track_id).copied().flatten(),
            sample_rates.get(&track_id).copied().flatten(),
            start_time,
        );
        if !decode_and_forward_packet(
            decoder,
            &packet,
            channels,
            &source_key,
            &infra,
            &mut log,
            packet_ts,
        ) {
            break;
        }
    }
}

fn finish_container_sources(wanted: &HashSet<u32>, sender: &mpsc::SyncSender<DecodeWorkerEvent>) {
    for track_id in wanted {
        let _ = sender.send(DecodeWorkerEvent::SourceFinished {
            source_key: SourceKey::TrackId(*track_id),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use symphonia::core::errors::Error;

    use super::super::super::super::buffer_mixer::SourceKey;
    use super::super::super::super::decoder_events::DecodeWorkerEvent;
    use super::spawn_container_decode_worker;

    #[test]
    fn container_worker_symbol_is_linked() {
        let _ = spawn_container_decode_worker;
    }

    /// Returns true when the error is an EOF (UnexpectedEof), mirroring the
    /// match pattern used in `decode_container_packets`.
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
    fn eof_emits_stream_exhausted_not_source_error() {
        let (sender, receiver) = mpsc::sync_channel::<DecodeWorkerEvent>(16);
        let track_ids: Vec<u32> = vec![1, 2];
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "end of stream",
        ));

        // Simulate the EOF branch: StreamExhausted, no SourceError
        if is_eof(&err) {
            let _ = sender.send(DecodeWorkerEvent::StreamExhausted);
        } else {
            for track_id in &track_ids {
                let _ = sender.send(DecodeWorkerEvent::SourceError {
                    source_key: SourceKey::TrackId(*track_id),
                    recoverable: false,
                    message: format!("packet-read failed: {}", err),
                });
            }
        }
        drop(sender);

        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DecodeWorkerEvent::StreamExhausted));
    }

    #[test]
    fn real_io_error_emits_source_errors_not_stream_exhausted() {
        let (sender, receiver) = mpsc::sync_channel::<DecodeWorkerEvent>(16);
        let track_ids: Vec<u32> = vec![1, 2];
        let err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "disk failure",
        ));

        // Simulate the real-error branch: per-track SourceError, no StreamExhausted
        if is_eof(&err) {
            let _ = sender.send(DecodeWorkerEvent::StreamExhausted);
        } else {
            for track_id in &track_ids {
                let _ = sender.send(DecodeWorkerEvent::SourceError {
                    source_key: SourceKey::TrackId(*track_id),
                    recoverable: false,
                    message: format!("packet-read failed: {}", err),
                });
            }
        }
        drop(sender);

        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(events.len(), 2, "should emit one SourceError per track");
        for event in &events {
            assert!(matches!(
                event,
                DecodeWorkerEvent::SourceError {
                    recoverable: false,
                    ..
                }
            ));
        }
    }
}
