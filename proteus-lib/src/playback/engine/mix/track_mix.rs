//! Per-chunk track mixing helpers.

use std::collections::HashMap;

use crate::audio::buffer::TrackBuffer;

use super::super::premix::PremixBuffer;

/// Arguments for mixing active/fading track buffers into the premix buffer.
pub(super) struct TrackMixArgs<'a> {
    pub(super) mix_buffer: &'a mut [f32],
    pub(super) premix_buffer: &'a mut PremixBuffer,
    pub(super) active_buffer_snapshot: &'a [(u16, TrackBuffer)],
    pub(super) fading_buffer_snapshot: &'a [(u16, TrackBuffer)],
    pub(super) weights_snapshot: &'a HashMap<u16, f32>,
    pub(super) channel_gains_snapshot: &'a HashMap<u16, Vec<f32>>,
    pub(super) fading_tracks: &'a mut HashMap<u16, (u32, u32)>,
    pub(super) min_mix_samples: usize,
    pub(super) premix_max_samples: usize,
    pub(super) all_tracks_finished: bool,
    pub(super) active_min_len: usize,
    pub(super) finished_min_len: usize,
    pub(super) next_event_ms: Option<u64>,
    pub(super) current_source_ms: u64,
    pub(super) sample_rate: u32,
    pub(super) channel_count: usize,
}

/// Mix a single chunk from active and fading track buffers.
///
/// # Returns
///
/// Number of source frames consumed and whether any work was performed.
pub(super) fn mix_tracks_into_premix(args: TrackMixArgs<'_>) -> (u64, bool) {
    let TrackMixArgs {
        mix_buffer,
        premix_buffer,
        active_buffer_snapshot,
        fading_buffer_snapshot,
        weights_snapshot,
        channel_gains_snapshot,
        fading_tracks,
        min_mix_samples,
        premix_max_samples,
        all_tracks_finished,
        active_min_len,
        finished_min_len,
        next_event_ms,
        current_source_ms,
        sample_rate,
        channel_count,
    } = args;

    let mut current_chunk = if !all_tracks_finished && active_min_len >= min_mix_samples {
        min_mix_samples
    } else if all_tracks_finished && finished_min_len > 0 {
        finished_min_len
    } else {
        0
    };

    if let Some(next_event_ms) = next_event_ms {
        if sample_rate > 0 && next_event_ms > current_source_ms {
            let remaining_ms = next_event_ms - current_source_ms;
            let frames_until_event = (remaining_ms.saturating_mul(sample_rate as u64)) / 1000;
            let samples_until_event = frames_until_event as usize * channel_count;
            if samples_until_event > 0 {
                current_chunk = current_chunk.min(samples_until_event);
            }
        }
    }

    let premix_room = premix_max_samples.saturating_sub(premix_buffer.len());
    current_chunk = current_chunk.min(premix_room).min(mix_buffer.len());

    if current_chunk == 0 {
        return (0, false);
    }

    mix_buffer.fill(0.0);

    for (track_key, buffer) in active_buffer_snapshot {
        let weight = weights_snapshot.get(track_key).copied().unwrap_or(1.0);
        let channel_gains = channel_gains_snapshot.get(track_key).map(Vec::as_slice);
        let mut buffer = buffer.lock().unwrap();
        let take = buffer.len().min(current_chunk);
        for (sample_index, sample) in mix_buffer.iter_mut().take(take).enumerate() {
            if let Some(value) = buffer.pop() {
                let gain = channel_gains
                    .and_then(|gains| gains.get(sample_index % channel_count))
                    .copied()
                    .unwrap_or(1.0);
                *sample += value * weight * gain;
            }
        }
    }

    if !fading_buffer_snapshot.is_empty() {
        let chunk_frames = (current_chunk / channel_count).max(1) as u32;
        for (track_key, buffer) in fading_buffer_snapshot {
            let Some((frames_remaining, total_frames)) = fading_tracks.get(track_key).copied()
            else {
                continue;
            };
            if total_frames == 0 {
                continue;
            }
            let weight = weights_snapshot.get(track_key).copied().unwrap_or(1.0);
            let channel_gains = channel_gains_snapshot.get(track_key).map(Vec::as_slice);
            let mut buffer = buffer.lock().unwrap();
            let take = buffer.len().min(current_chunk);
            for (sample_index, sample) in mix_buffer.iter_mut().take(take).enumerate() {
                let Some(value) = buffer.pop() else {
                    continue;
                };
                let frame_index = (sample_index / channel_count) as u32;
                if frame_index >= frames_remaining {
                    continue;
                }
                let fade_gain =
                    frames_remaining.saturating_sub(frame_index) as f32 / total_frames as f32;
                let gain = channel_gains
                    .and_then(|gains| gains.get(sample_index % channel_count))
                    .copied()
                    .unwrap_or(1.0);
                *sample += value * weight * gain * fade_gain;
            }
            if let Some((remaining, _)) = fading_tracks.get_mut(track_key) {
                *remaining = remaining.saturating_sub(chunk_frames);
            }
        }
        fading_tracks.retain(|_, (remaining, _)| *remaining > 0);
    }

    premix_buffer.push_interleaved(&mix_buffer[..current_chunk]);
    let consumed_source_frames = current_chunk as u64 / channel_count as u64;
    (consumed_source_frames, true)
}
