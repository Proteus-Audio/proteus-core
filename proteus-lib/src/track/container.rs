//! Buffering implementation for multiple tracks in a shared container stream.

use log::warn;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{SeekMode, SeekTo};
use symphonia::core::units::{Time, TimeBase};

use crate::audio::buffer::TrackBufferMap;

use super::buffer::{add_samples_to_buffer_map, mark_track_as_finished};
use super::convert::process_channel;

/// Arguments required to buffer multiple tracks from a shared container stream.
pub struct ContainerTrackArgs {
    pub file_path: String,
    pub track_entries: Vec<(u16, u32)>,
    pub buffer_map: TrackBufferMap,
    pub buffer_notify: Option<Arc<std::sync::Condvar>>,
    pub track_weights: Option<Arc<Mutex<HashMap<u16, f32>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
    pub track_eos_ms: f32,
}

/// Spawn a decoder thread that buffers multiple container tracks.
pub fn buffer_container_tracks(
    args: ContainerTrackArgs,
    abort: Arc<AtomicBool>,
) -> Arc<Mutex<bool>> {
    let ContainerTrackArgs {
        file_path,
        track_entries,
        buffer_map,
        buffer_notify,
        track_weights,
        finished_tracks,
        start_time,
        channels,
        track_eos_ms,
    } = args;

    let (
        mut format,
        mut decoders,
        durations,
        time_bases,
        sample_rates,
        keys_for_track,
        valid_entries,
    ) = {
        let format = crate::tools::tools::get_reader(&file_path);
        let mut decoders: HashMap<u32, Box<dyn Decoder>> = HashMap::new();
        let mut durations: HashMap<u32, Option<u64>> = HashMap::new();
        let mut time_bases: HashMap<u32, Option<TimeBase>> = HashMap::new();
        let mut sample_rates: HashMap<u32, Option<u32>> = HashMap::new();
        let mut keys_for_track: HashMap<u32, Vec<u16>> = HashMap::new();
        let mut valid_entries: Vec<(u16, u32)> = Vec::new();

        for (track_key, track_id) in &track_entries {
            let track = match format.tracks().iter().find(|track| track.id == *track_id) {
                Some(track) => track,
                None => {
                    warn!("container track missing: id {}", track_id);
                    mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
                    if let Some(notify) = buffer_notify.as_ref() {
                        notify.notify_all();
                    }
                    continue;
                }
            };

            if !decoders.contains_key(track_id) {
                let dec_opts: DecoderOptions = Default::default();
                let decoder = symphonia::default::get_codecs()
                    .make(&track.codec_params, &dec_opts)
                    .expect("unsupported codec");
                decoders.insert(*track_id, decoder);
            }

            let dur = track
                .codec_params
                .n_frames
                .map(|frames| track.codec_params.start_ts + frames);
            durations.insert(*track_id, dur);
            time_bases.insert(*track_id, track.codec_params.time_base);
            sample_rates.insert(*track_id, track.codec_params.sample_rate);
            keys_for_track
                .entry(*track_id)
                .or_default()
                .push(*track_key);
            valid_entries.push((*track_key, *track_id));
        }

        (
            format,
            decoders,
            durations,
            time_bases,
            sample_rates,
            keys_for_track,
            valid_entries,
        )
    };

    let playing: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

    thread::spawn(move || {
        if valid_entries.is_empty() {
            warn!("no valid tracks found in container");
            for (track_key, _) in &track_entries {
                mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
            }
            return;
        }

        if let Some(weights) = &track_weights {
            let mut weights = weights.lock().unwrap();
            for (_, keys) in keys_for_track.iter() {
                if let Some(primary_key) = keys.first() {
                    let count = keys.len() as f32;
                    weights.insert(*primary_key, count);
                    for dup_key in keys.iter().skip(1) {
                        weights.insert(*dup_key, 0.0);
                        mark_track_as_finished(&mut finished_tracks.clone(), *dup_key);
                    }
                }
            }
        }

        let seconds = start_time.floor() as u64;
        let frac_of_second = start_time.fract();
        let time = Time::new(seconds, frac_of_second);

        let seek_track_id = valid_entries[0].1;
        let seek_success = format.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time,
                track_id: Some(seek_track_id),
            },
        );

        if seek_success.is_err() {
            warn!("container seek failed, starting from beginning");
        }

        let mut finished_track_ids: Vec<u32> = Vec::new();
        let mut last_seen_secs: HashMap<u32, f64> = HashMap::new();
        let mut max_seen_secs: f64 = 0.0;
        let eos_seconds = (track_eos_ms.max(0.0) / 1000.0) as f64;

        let _result: Result<bool, Error> = loop {
            if abort.load(std::sync::atomic::Ordering::Relaxed) {
                break Ok(true);
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(err) => break Err(err),
            };

            let track_id = packet.track_id();
            let track_keys = match keys_for_track.get(&track_id) {
                Some(track_keys) => track_keys.clone(),
                None => continue,
            };
            if track_keys.is_empty() {
                continue;
            }
            let primary_track_key = track_keys[0];

            if let Some(time_base) = time_bases.get(&track_id).copied().flatten() {
                let time = time_base.calc_time(packet.ts());
                let secs = time.seconds as f64 + time.frac;
                last_seen_secs.insert(track_id, secs);
                if secs > max_seen_secs {
                    max_seen_secs = secs;
                }
            } else if let Some(sample_rate) = sample_rates.get(&track_id).copied().flatten() {
                let secs = packet.ts() as f64 / sample_rate as f64;
                last_seen_secs.insert(track_id, secs);
                if secs > max_seen_secs {
                    max_seen_secs = secs;
                }
            }

            if let Some(dur) = durations.get(&track_id).copied().flatten() {
                if packet.ts() >= dur {
                    if !finished_track_ids.contains(&track_id) {
                        finished_track_ids.push(track_id);
                        mark_track_as_finished(&mut finished_tracks.clone(), primary_track_key);
                    }
                    if finished_track_ids.len() == keys_for_track.len() {
                        break Ok(true);
                    }
                    continue;
                }
            } else {
                warn!("Track {} has no duration", primary_track_key);
            }

            if eos_seconds > 0.0 && max_seen_secs > 0.0 {
                for (candidate_track_id, keys) in keys_for_track.iter() {
                    if finished_track_ids.contains(candidate_track_id) {
                        continue;
                    }
                    if let Some(last_seen) = last_seen_secs.get(candidate_track_id).copied() {
                        if max_seen_secs - last_seen >= eos_seconds {
                            if let Some(primary_key) = keys.first() {
                                finished_track_ids.push(*candidate_track_id);
                                mark_track_as_finished(&mut finished_tracks.clone(), *primary_key);
                                if let Some(notify) = buffer_notify.as_ref() {
                                    notify.notify_all();
                                }
                            }
                        }
                    }
                }
            }

            let decoder = match decoders.get_mut(&track_id) {
                Some(decoder) => decoder,
                None => continue,
            };

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
                        primary_track_key,
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

        for (track_key, track_id) in &valid_entries {
            if !finished_track_ids.contains(track_id) {
                mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
            }
        }
    });

    playing
}
