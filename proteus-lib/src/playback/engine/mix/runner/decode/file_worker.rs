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
use super::{forward_decoded_packet, interleaved_samples, packet_ts_seconds};

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
    decode_file_packets(
        &mut decoder,
        format.as_mut(),
        &track,
        start_time,
        channels,
        &source_key,
        &sender,
        abort.as_ref(),
        decode_backpressure.as_ref(),
        startup_trace,
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

#[allow(clippy::too_many_arguments)]
fn decode_file_packets(
    decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
    format: &mut dyn symphonia::core::formats::FormatReader,
    track: &symphonia::core::formats::Track,
    start_time: f64,
    channels: u8,
    source_key: &SourceKey,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
    abort: &std::sync::atomic::AtomicBool,
    decode_backpressure: &DecodeBackpressure,
    startup_trace: Instant,
) {
    let mut logged_first_ready = false;
    let mut logged_first_send = false;
    let time_base = track.codec_params.time_base;
    let sample_rate = track.codec_params.sample_rate;
    loop {
        if abort.load(Ordering::Relaxed) {
            break;
        }

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }

        let packet_ts = packet_ts_seconds(packet.ts(), time_base, sample_rate, start_time);
        if !decode_and_forward_file_packet(
            decoder,
            &packet,
            channels,
            source_key,
            sender,
            abort,
            decode_backpressure,
            startup_trace,
            &mut logged_first_ready,
            &mut logged_first_send,
            packet_ts,
        ) {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_and_forward_file_packet(
    decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
    packet: &symphonia::core::formats::Packet,
    channels: u8,
    source_key: &SourceKey,
    sender: &mpsc::SyncSender<DecodeWorkerEvent>,
    abort: &std::sync::atomic::AtomicBool,
    decode_backpressure: &DecodeBackpressure,
    startup_trace: Instant,
    logged_first_ready: &mut bool,
    logged_first_send: &mut bool,
    packet_ts: f64,
) -> bool {
    match decoder.decode(packet) {
        Ok(decoded) => {
            let samples = interleaved_samples(decoded, channels);
            if samples.is_empty() {
                return true;
            }
            forward_decoded_packet(
                "file",
                source_key.clone(),
                packet_ts,
                samples,
                sender,
                decode_backpressure,
                abort,
                startup_trace,
                logged_first_ready,
                logged_first_send,
            )
        }
        Err(Error::DecodeError(err)) => {
            let _ = sender.send(DecodeWorkerEvent::SourceError {
                source_key: source_key.clone(),
                recoverable: true,
                message: err.to_string(),
            });
            true
        }
        Err(err) => {
            let _ = sender.send(DecodeWorkerEvent::SourceError {
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
    use super::spawn_file_decode_worker;

    #[test]
    fn file_worker_symbol_is_linked() {
        let _ = spawn_file_decode_worker;
    }
}
