//! Standalone-file decode worker.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use log::{debug, info};
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::Time;

use crate::tools::tools::open_file;

use super::super::super::buffer_mixer::{DecodeBackpressure, SourceKey};
use super::super::super::decoder_events::DecodedPacket;
use super::{interleaved_samples, packet_ts_seconds};

/// Spawn a decode worker for one standalone audio file source.
pub(crate) fn spawn_file_decode_worker(
    file_path: String,
    start_time: f64,
    channels: u8,
    sender: mpsc::SyncSender<Option<DecodedPacket>>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    decode_backpressure: Arc<DecodeBackpressure>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let startup_trace = Instant::now();
        let mut logged_first_ready = false;
        let mut logged_first_send = false;
        let (mut decoder, mut format) = open_file(&file_path);
        let Some(track) = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .cloned()
        else {
            let _ = sender.send(Some(DecodedPacket {
                source_key: SourceKey::FilePath(file_path),
                packet_ts: 0.0,
                samples: Vec::new(),
                eos_flag: true,
            }));
            return;
        };

        let seconds = start_time.floor() as u64;
        let frac_of_second = start_time.fract();
        let time = Time::new(seconds, frac_of_second);
        let _ = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time,
                track_id: Some(track.id),
            },
        );

        let time_base = track.codec_params.time_base;
        let sample_rate = track.codec_params.sample_rate;
        let source_key = SourceKey::FilePath(file_path.clone());

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
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let samples = interleaved_samples(decoded, channels);
                    if samples.is_empty() {
                        continue;
                    }
                    debug!(
                        "file decode packet ready: source={:?} ts={:.6} samples={}",
                        source_key,
                        packet_ts,
                        samples.len()
                    );
                    if !logged_first_ready {
                        logged_first_ready = true;
                        info!(
                            "mix startup trace: file worker first decoded packet ready in {}ms (source={:?} ts={:.6} samples={})",
                            startup_trace.elapsed().as_millis(),
                            source_key,
                            packet_ts,
                            samples.len()
                        );
                    }
                    if !decode_backpressure.wait_for_source_room(
                        &source_key,
                        samples.len(),
                        abort.as_ref(),
                    ) {
                        debug!(
                            "file decode wait interrupted: source={:?} ts={:.6} samples={}",
                            source_key,
                            packet_ts,
                            samples.len()
                        );
                        break;
                    }
                    debug!(
                        "file decode packet send: source={:?} ts={:.6} samples={}",
                        source_key,
                        packet_ts,
                        samples.len()
                    );
                    if sender
                        .send(Some(DecodedPacket {
                            source_key: source_key.clone(),
                            packet_ts,
                            samples,
                            eos_flag: false,
                        }))
                        .is_err()
                    {
                        break;
                    } else if !logged_first_send {
                        logged_first_send = true;
                        info!(
                            "mix startup trace: file worker first packet sent in {}ms (source={:?})",
                            startup_trace.elapsed().as_millis(),
                            source_key
                        );
                    }
                }
                Err(Error::DecodeError(_)) => {}
                Err(_) => break,
            }
        }

        let _ = sender.send(Some(DecodedPacket {
            source_key,
            packet_ts: 0.0,
            samples: Vec::new(),
            eos_flag: true,
        }));
    })
}
