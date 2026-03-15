//! Buffering implementation for multiple tracks in a shared container stream.
//!
//! This is a legacy buffering module retained for reference. It is not wired
//! into the active playback engine.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatReader, Packet, SeekMode, SeekTo};
use symphonia::core::units::{Time, TimeBase};

use log::{info, warn};

use crate::audio::buffer::TrackBufferMap;
use crate::audio::decode::process_channel;

use super::buffer::{add_samples_to_buffer_map, mark_track_as_finished};

/// Arguments required to buffer multiple tracks from a shared container stream.
pub struct ContainerTrackArgs {
    pub file_path: String,
    pub track_entries: Vec<(u16, u32)>,
    pub buffer_map: TrackBufferMap,
    pub buffer_notify: Option<Arc<Condvar>>,
    pub track_weights: Option<Arc<Mutex<HashMap<u16, f32>>>>,
    pub finished_tracks: Arc<Mutex<Vec<u16>>>,
    pub start_time: f64,
    pub channels: u8,
    pub track_eos_ms: f32,
}

struct TrackDecoder {
    track_id: u32,
    track_keys: Vec<u16>,
    decoder: Box<dyn Decoder>,
    duration: Option<u64>,
    time_base: Option<TimeBase>,
    sample_rate: Option<u32>,
}

impl TrackDecoder {
    fn primary_key(&self) -> u16 {
        self.track_keys[0]
    }

    fn packet_position_secs(&self, packet: &Packet) -> Option<f64> {
        if let Some(tb) = self.time_base {
            let t = tb.calc_time(packet.ts());
            return Some(t.seconds as f64 + t.frac);
        }
        self.sample_rate.map(|sr| packet.ts() as f64 / sr as f64)
    }

    fn is_past_end(&self, packet: &Packet) -> bool {
        self.duration.is_some_and(|dur| packet.ts() >= dur)
    }
}

/// Spawn a decoder thread that buffers multiple container tracks.
pub fn buffer_container_tracks(args: ContainerTrackArgs, abort: Arc<AtomicBool>) -> JoinHandle<()> {
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
    let mut format = match crate::tools::decode::get_reader(&file_path) {
        Ok(f) => f,
        Err(err) => {
            return thread::spawn(move || {
                warn!("failed to open container '{}': {}", file_path, err);
                for (track_key, _) in &track_entries {
                    mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
                }
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
            });
        }
    };
    let mut track_decoders = open_container_decoders(
        &mut *format,
        &track_entries,
        &finished_tracks,
        buffer_notify.as_ref(),
    );
    thread::spawn(move || {
        if track_decoders.is_empty() {
            warn!("no valid tracks found in container");
            for (track_key, _) in &track_entries {
                mark_track_as_finished(&mut finished_tracks.clone(), *track_key);
            }
            return;
        }
        if let Some(weights) = &track_weights {
            init_container_weights(&track_decoders, &finished_tracks, weights);
        }
        let time = Time::new(start_time.floor() as u64, start_time.fract());
        if format
            .seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time,
                    track_id: Some(track_decoders[0].track_id),
                },
            )
            .is_err()
        {
            warn!("container seek failed, starting from beginning");
        }
        let eos_seconds = (track_eos_ms.max(0.0) / 1000.0) as f64;
        let finished_ids = run_container_decode_loop(
            &mut *format,
            &mut track_decoders,
            channels,
            eos_seconds,
            &buffer_map,
            buffer_notify.as_ref(),
            &abort,
            &finished_tracks,
        );
        for td in &track_decoders {
            if !finished_ids.contains(&td.track_id) {
                for &key in &td.track_keys {
                    mark_track_as_finished(&mut finished_tracks.clone(), key);
                }
                if let Some(notify) = buffer_notify.as_ref() {
                    notify.notify_all();
                }
            }
        }
    })
}

fn open_container_decoders(
    format: &mut dyn FormatReader,
    track_entries: &[(u16, u32)],
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
    buffer_notify: Option<&Arc<Condvar>>,
) -> Vec<TrackDecoder> {
    let mut result: Vec<TrackDecoder> = Vec::new();
    for &(track_key, track_id) in track_entries {
        let duplicate = result.iter().position(|td| td.track_id == track_id);
        if let Some(idx) = duplicate {
            result[idx].track_keys.push(track_key);
            continue;
        }
        let Some(track) = format.tracks().iter().find(|t| t.id == track_id) else {
            warn!("container track missing: id {}", track_id);
            mark_track_as_finished(&mut finished_tracks.clone(), track_key);
            if let Some(notify) = buffer_notify {
                notify.notify_all();
            }
            continue;
        };
        let decoder = match symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
        {
            Ok(d) => d,
            Err(e) => {
                warn!("unsupported codec for track {}: {}", track_id, e);
                mark_track_as_finished(&mut finished_tracks.clone(), track_key);
                if let Some(notify) = buffer_notify {
                    notify.notify_all();
                }
                continue;
            }
        };
        let duration = track
            .codec_params
            .n_frames
            .map(|f| track.codec_params.start_ts + f);
        result.push(TrackDecoder {
            track_id,
            track_keys: vec![track_key],
            decoder,
            duration,
            time_base: track.codec_params.time_base,
            sample_rate: track.codec_params.sample_rate,
        });
    }
    result
}

fn init_container_weights(
    track_decoders: &[TrackDecoder],
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
    weights: &Arc<Mutex<HashMap<u16, f32>>>,
) {
    let mut w = weights.lock().unwrap_or_else(|_| {
        panic!("track weights lock poisoned — a thread panicked while holding it")
    });
    for td in track_decoders {
        let count = td.track_keys.len() as f32;
        if let Some(&primary) = td.track_keys.first() {
            w.insert(primary, count);
            for &dup_key in td.track_keys.iter().skip(1) {
                w.insert(dup_key, 0.0);
                mark_track_as_finished(&mut finished_tracks.clone(), dup_key);
            }
        }
    }
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

fn check_eos_skew(
    track_decoders: &[TrackDecoder],
    finished_ids: &mut Vec<u32>,
    last_seen: &HashMap<u32, f64>,
    max_seen: f64,
    eos_seconds: f64,
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
    buffer_notify: Option<&Arc<Condvar>>,
) {
    if eos_seconds <= 0.0 || max_seen <= 0.0 {
        return;
    }
    for td in track_decoders {
        if finished_ids.contains(&td.track_id) {
            continue;
        }
        let Some(&last) = last_seen.get(&td.track_id) else {
            continue;
        };
        if max_seen - last >= eos_seconds {
            finished_ids.push(td.track_id);
            mark_track_as_finished(&mut finished_tracks.clone(), td.primary_key());
            if let Some(notify) = buffer_notify {
                notify.notify_all();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_decoded_container_packet(
    track_id: u32,
    decoded: AudioBufferRef<'_>,
    primary_key: u16,
    channels: u8,
    buffer_map: &TrackBufferMap,
    buffer_notify: Option<&Arc<Condvar>>,
    abort: &Arc<AtomicBool>,
    logged_formats: &mut HashMap<u32, bool>,
) {
    logged_formats.entry(track_id).or_insert_with(|| {
        info!("decoded track {} buffer format logged", track_id);
        true
    });
    let stereo = interleave_to_stereo(decoded, channels);
    if !stereo.is_empty() {
        add_samples_to_buffer_map(
            &mut buffer_map.clone(),
            primary_key,
            stereo,
            abort,
            buffer_notify,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn run_container_decode_loop(
    format: &mut dyn FormatReader,
    track_decoders: &mut [TrackDecoder],
    channels: u8,
    eos_seconds: f64,
    buffer_map: &TrackBufferMap,
    buffer_notify: Option<&Arc<Condvar>>,
    abort: &Arc<AtomicBool>,
    finished_tracks: &Arc<Mutex<Vec<u16>>>,
) -> Vec<u32> {
    let mut finished_ids: Vec<u32> = Vec::new();
    let mut last_seen: HashMap<u32, f64> = HashMap::new();
    let (mut max_seen, mut logged_formats) = (0.0f64, HashMap::<u32, bool>::new());
    let result: Result<bool, Error> = loop {
        if abort.load(Ordering::Relaxed) {
            break Ok(true);
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(e) => break Err(e),
        };
        let tid = packet.track_id();
        let Some(td) = track_decoders.iter_mut().find(|td| td.track_id == tid) else {
            continue;
        };
        if let Some(s) = td.packet_position_secs(&packet) {
            last_seen.insert(tid, s);
            max_seen = max_seen.max(s);
        }
        let pkey = td.primary_key();
        if td.is_past_end(&packet) {
            if !finished_ids.contains(&tid) {
                finished_ids.push(tid);
                mark_track_as_finished(&mut finished_tracks.clone(), pkey);
            }
            if finished_ids.len() == track_decoders.len() {
                break Ok(true);
            }
            continue;
        }
        let _ = td;
        check_eos_skew(
            track_decoders,
            &mut finished_ids,
            &last_seen,
            max_seen,
            eos_seconds,
            finished_tracks,
            buffer_notify,
        );
        // The decoder for `tid` was found at the top of this iteration;
        // `check_eos_skew` does not remove decoders, so it must still be present.
        let td = track_decoders
            .iter_mut()
            .find(|td| td.track_id == tid)
            .unwrap_or_else(|| {
                unreachable!("decoder for tid {} missing after check_eos_skew", tid)
            });
        let tid_for_log = td.track_id;
        match td.decoder.decode(&packet) {
            Ok(decoded) => push_decoded_container_packet(
                tid_for_log,
                decoded,
                pkey,
                channels,
                buffer_map,
                buffer_notify,
                abort,
                &mut logged_formats,
            ),
            Err(Error::DecodeError(e)) => warn!("decode error: {}", e),
            Err(e) => break Err(e),
        }
    };
    if let Err(e) = result {
        warn!("error: {}", e);
    }
    finished_ids
}

#[cfg(test)]
mod tests {
    use super::{buffer_container_tracks, ContainerTrackArgs};
    use crate::audio::buffer::init_buffer_map;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    #[test]
    fn buffer_container_tracks_marks_all_finished_when_open_fails() {
        let finished_tracks = Arc::new(Mutex::new(Vec::new()));
        let args = ContainerTrackArgs {
            file_path: "/definitely/missing/container.mka".to_string(),
            track_entries: vec![(1, 10), (2, 11)],
            buffer_map: init_buffer_map(),
            buffer_notify: None,
            track_weights: Some(Arc::new(Mutex::new(HashMap::new()))),
            finished_tracks: finished_tracks.clone(),
            start_time: 0.0,
            channels: 2,
            track_eos_ms: 0.0,
        };
        let abort = Arc::new(AtomicBool::new(false));

        let handle = buffer_container_tracks(args, abort);
        handle.join().expect("worker thread should complete");

        let done = finished_tracks.lock().unwrap().clone();
        assert!(done.contains(&1));
        assert!(done.contains(&2));
    }
}
