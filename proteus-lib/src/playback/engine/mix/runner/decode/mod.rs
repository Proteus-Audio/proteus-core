//! Decode worker helpers used by the mix thread.

mod container_worker;
mod file_worker;

use std::thread::JoinHandle;

use log::{debug, info, warn};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::units::TimeBase;

use super::super::buffer_mixer::SourceKey;
use super::super::decoder_events::{DecodeWorkerEvent, DecodedPacket};

pub(super) use container_worker::spawn_container_decode_worker;
pub(super) use file_worker::spawn_file_decode_worker;

/// Shared decode-worker context passed to `forward_decoded_packet`.
pub(super) struct ForwardInfra<'a> {
    pub sender: &'a std::sync::mpsc::SyncSender<super::super::decoder_events::DecodeWorkerEvent>,
    pub decode_backpressure: &'a super::super::buffer_mixer::DecodeBackpressure,
    pub abort: &'a std::sync::atomic::AtomicBool,
    pub startup_trace: std::time::Instant,
}

/// Per-worker startup logging state.
pub(super) struct StartupLog {
    pub logged_first_ready: bool,
    pub logged_first_send: bool,
}

/// Ensures decode workers are joined during mix-thread teardown.
#[derive(Default)]
pub(super) struct DecodeWorkerJoinGuard {
    workers: Vec<JoinHandle<()>>,
}

impl DecodeWorkerJoinGuard {
    /// Register a decode worker to be joined during teardown.
    pub(super) fn push(&mut self, handle: JoinHandle<()>) {
        self.workers.push(handle);
    }
}

impl Drop for DecodeWorkerJoinGuard {
    /// Join all registered decode workers, logging but tolerating panics.
    fn drop(&mut self) {
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                warn!("decode worker panicked during join");
            }
        }
    }
}

/// Convert a decoded packet into stereo interleaved samples for the mixer.
pub(super) fn interleaved_samples(decoded: AudioBufferRef<'_>, channels: u8) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let mut out_channels: Vec<Vec<f32>> = Vec::with_capacity(channels);
    for channel in 0..channels {
        out_channels.push(crate::audio::decode::process_channel(
            decoded.clone(),
            channel,
        ));
    }

    if out_channels.is_empty() {
        return Vec::new();
    }

    let left = out_channels[0].clone();
    let right = out_channels
        .get(1)
        .cloned()
        .unwrap_or_else(|| out_channels[0].clone());

    left.into_iter()
        .zip(right)
        .flat_map(|(l, r)| [l, r])
        .collect()
}

/// Convert packet timestamp units to a seek-relative seconds value.
pub(super) fn packet_ts_seconds(
    ts: u64,
    time_base: Option<TimeBase>,
    sample_rate: Option<u32>,
    start_time: f64,
) -> f64 {
    let absolute = if let Some(base) = time_base {
        let time = base.calc_time(ts);
        time.seconds as f64 + time.frac
    } else if let Some(rate) = sample_rate {
        ts as f64 / rate.max(1) as f64
    } else {
        0.0
    };
    (absolute - start_time).max(0.0)
}

/// Shared decode-worker path: apply backpressure and forward one packet.
pub(super) fn forward_decoded_packet(
    worker_label: &str,
    source_key: SourceKey,
    packet_ts: f64,
    samples: Vec<f32>,
    infra: &ForwardInfra<'_>,
    log: &mut StartupLog,
) -> bool {
    debug!(
        "{} decode packet ready: source={:?} ts={:.6} samples={}",
        worker_label,
        source_key,
        packet_ts,
        samples.len()
    );
    if !log.logged_first_ready {
        log.logged_first_ready = true;
        info!(
            "mix startup trace: {} worker first decoded packet ready in {}ms (source={:?} ts={:.6} samples={})",
            worker_label,
            infra.startup_trace.elapsed().as_millis(),
            source_key,
            packet_ts,
            samples.len()
        );
    }

    if !infra
        .decode_backpressure
        .wait_for_source_room(&source_key, samples.len(), infra.abort)
    {
        debug!(
            "{} decode wait interrupted: source={:?} ts={:.6} samples={}",
            worker_label,
            source_key,
            packet_ts,
            samples.len()
        );
        return false;
    }

    debug!(
        "{} decode packet send: source={:?} ts={:.6} samples={}",
        worker_label,
        source_key,
        packet_ts,
        samples.len()
    );
    if infra
        .sender
        .send(DecodeWorkerEvent::Packet(DecodedPacket {
            source_key: source_key.clone(),
            packet_ts,
            samples,
        }))
        .is_err()
    {
        return false;
    }
    if !log.logged_first_send {
        log.logged_first_send = true;
        info!(
            "mix startup trace: {} worker first packet sent in {}ms (source={:?})",
            worker_label,
            infra.startup_trace.elapsed().as_millis(),
            source_key
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use symphonia::core::units::TimeBase;

    #[test]
    fn packet_ts_seconds_uses_sample_rate_fallback() {
        let ts = packet_ts_seconds(48_000, None, Some(48_000), 0.5);
        assert!((ts - 0.5).abs() < 1e-6);
    }

    #[test]
    fn packet_ts_seconds_clamps_to_zero() {
        let ts = packet_ts_seconds(0, Some(TimeBase::new(1, 1)), None, 2.0);
        assert_eq!(ts, 0.0);
    }

    #[test]
    fn decode_worker_join_guard_joins_registered_threads() {
        let joined = Arc::new(Mutex::new(0_u32));
        let joined_clone = joined.clone();
        let handle = thread::spawn(move || {
            *joined_clone.lock().unwrap() += 1;
        });

        let mut guard = DecodeWorkerJoinGuard::default();
        guard.push(handle);
        drop(guard);

        assert_eq!(*joined.lock().unwrap(), 1);
    }
}
