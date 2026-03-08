//! Container demux decode worker (single demuxer feeding multiple track decoders).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use log::{debug, error, info};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::{Time, TimeBase};

use super::super::super::buffer_mixer::{DecodeBackpressure, SourceKey};
use super::super::super::decoder_events::DecodedPacket;
use super::{interleaved_samples, packet_ts_seconds};

/// Spawn a single demux decode worker that services multiple container track ids.
pub(crate) fn spawn_container_decode_worker(
    file_path: String,
    track_ids: Vec<u32>,
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
        let mut format = crate::tools::tools::get_reader(&file_path);
        let mut decoders: HashMap<u32, Box<dyn Decoder>> = HashMap::new();
        let mut time_bases: HashMap<u32, Option<TimeBase>> = HashMap::new();
        let mut sample_rates: HashMap<u32, Option<u32>> = HashMap::new();
        let wanted: HashSet<u32> = track_ids.iter().copied().collect();

        for track_id in wanted.iter().copied() {
            let Some(track) = format.tracks().iter().find(|track| track.id == track_id) else {
                continue;
            };
            let dec_opts: DecoderOptions = Default::default();
            if let Ok(decoder) =
                symphonia::default::get_codecs().make(&track.codec_params, &dec_opts)
            {
                decoders.insert(track_id, decoder);
                time_bases.insert(track_id, track.codec_params.time_base);
                sample_rates.insert(track_id, track.codec_params.sample_rate);
            }
        }

        if decoders.is_empty() {
            for track_id in wanted {
                let _ = sender.send(Some(DecodedPacket {
                    source_key: SourceKey::TrackId(track_id),
                    packet_ts: 0.0,
                    samples: Vec::new(),
                    eos_flag: true,
                }));
            }
            return;
        }

        if let Some(first_track_id) = decoders.keys().next().copied() {
            let seconds = start_time.floor() as u64;
            let frac_of_second = start_time.fract();
            let time = Time::new(seconds, frac_of_second);
            let _ = format.seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time,
                    track_id: Some(first_track_id),
                },
            );
        }

        loop {
            if abort.load(Ordering::Relaxed) {
                break;
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(_) => {
                    let _ = sender.send(None);
                    error!("No packet received! End?");
                    break;
                }
            };
            let track_id = packet.track_id();
            let Some(decoder) = decoders.get_mut(&track_id) else {
                continue;
            };

            let packet_ts = packet_ts_seconds(
                packet.ts(),
                time_bases.get(&track_id).copied().flatten(),
                sample_rates.get(&track_id).copied().flatten(),
                start_time,
            );
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let samples = interleaved_samples(decoded, channels);
                    if samples.is_empty() {
                        continue;
                    }
                    let source_key = SourceKey::TrackId(track_id);
                    debug!(
                        "container decode packet ready: source={:?} ts={:.6} samples={}",
                        source_key,
                        packet_ts,
                        samples.len()
                    );
                    if !logged_first_ready {
                        logged_first_ready = true;
                        info!(
                            "mix startup trace: container worker first decoded packet ready in {}ms (track_id={} ts={:.6} samples={})",
                            startup_trace.elapsed().as_millis(),
                            track_id,
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
                            "container decode wait interrupted: source={:?} ts={:.6} samples={}",
                            source_key,
                            packet_ts,
                            samples.len()
                        );
                        break;
                    }
                    debug!(
                        "container decode packet send: source={:?} ts={:.6} samples={}",
                        source_key,
                        packet_ts,
                        samples.len()
                    );
                    if sender
                        .send(Some(DecodedPacket {
                            source_key,
                            packet_ts,
                            samples,
                            eos_flag: false,
                        }))
                        .is_err()
                    {
                        error!("Breaking! End?");
                        break;
                    } else if !logged_first_send {
                        logged_first_send = true;
                        info!(
                            "mix startup trace: container worker first packet sent in {}ms (track_id={})",
                            startup_trace.elapsed().as_millis(),
                            track_id
                        );
                    }
                }
                Err(Error::DecodeError(_)) => {
                    error!("Failed to decode packet");
                }
                Err(_) => {
                    error!("No packet received. End?");
                    break;
                }
            }
        }

        for track_id in wanted {
            let _ = sender.send(Some(DecodedPacket {
                source_key: SourceKey::TrackId(track_id),
                packet_ts: 0.0,
                samples: Vec::new(),
                eos_flag: true,
            }));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::spawn_container_decode_worker;

    #[test]
    fn container_worker_symbol_is_linked() {
        let _ = spawn_container_decode_worker;
    }
}
