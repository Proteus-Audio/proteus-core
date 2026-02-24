//! Decode worker helpers used by the mix thread.

mod container_worker;
mod file_worker;

use std::thread::JoinHandle;

use log::warn;
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::units::TimeBase;

pub(super) use container_worker::spawn_container_decode_worker;
pub(super) use file_worker::spawn_file_decode_worker;

/// Ensures decode workers are joined during mix-thread teardown.
#[derive(Default)]
pub(super) struct DecodeWorkerJoinGuard {
    workers: Vec<JoinHandle<()>>,
}

impl DecodeWorkerJoinGuard {
    pub(super) fn push(&mut self, handle: JoinHandle<()>) {
        self.workers.push(handle);
    }
}

impl Drop for DecodeWorkerJoinGuard {
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
        out_channels.push(crate::track::convert::process_channel(
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
