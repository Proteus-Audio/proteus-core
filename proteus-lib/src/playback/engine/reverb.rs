//! Reverb worker and impulse-response resolution helpers.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
#[cfg(feature = "debug")]
use std::time::Instant;

#[cfg(feature = "debug")]
use log::info;
use log::warn;

use crate::dsp::effects::convolution_reverb::impulse_response::{
    load_impulse_response_from_file_with_tail, load_impulse_response_from_prot_attachment_with_tail,
};
use crate::dsp::effects::convolution_reverb::reverb::Reverb;
use crate::dsp::effects::convolution_reverb::ImpulseResponseSpec;

use super::state::{ReverbMetrics, ReverbSettings};

/// Build a [`Reverb`] instance based on an optional impulse response spec.
pub fn build_reverb_with_impulse_response(
    channels: usize,
    dry_wet: f32,
    impulse_spec: Option<ImpulseResponseSpec>,
    container_path: Option<&str>,
    tail_db: f32,
) -> Reverb {
    let impulse_spec = match impulse_spec {
        Some(spec) => spec,
        None => return Reverb::new(channels, dry_wet),
    };

    let result = match impulse_spec {
        ImpulseResponseSpec::Attachment(name) => container_path
            .ok_or_else(|| "missing container path for attachment".to_string())
            .and_then(|path| {
                load_impulse_response_from_prot_attachment_with_tail(path, &name, Some(tail_db))
                    .map_err(|err| err.to_string())
            }),
        ImpulseResponseSpec::FilePath(path) => {
            let resolved_path = resolve_impulse_response_path(container_path, &path);
            if resolved_path.exists() {
                load_impulse_response_from_file_with_tail(&resolved_path, Some(tail_db))
                    .map_err(|err| err.to_string())
            } else {
                match container_path {
                    Some(container_path) => {
                        let fallback_name = Path::new(&path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.to_string());
                        if let Some(fallback_name) = fallback_name {
                            load_impulse_response_from_prot_attachment_with_tail(
                                container_path,
                                &fallback_name,
                                Some(tail_db),
                            )
                            .map_err(|err| err.to_string())
                        } else {
                            Err(format!(
                                "impulse response path not found: {}",
                                resolved_path.display()
                            ))
                        }
                    }
                    None => Err(format!(
                        "impulse response path not found: {}",
                        resolved_path.display()
                    )),
                }
            }
        }
    };

    match result {
        Ok(impulse_response) => {
            Reverb::new_with_impulse_response(channels, dry_wet, &impulse_response)
        }
        Err(err) => {
            warn!(
                "Failed to load impulse response ({}); falling back to default reverb.",
                err
            );
            Reverb::new(channels, dry_wet)
        }
    }
}

fn resolve_impulse_response_path(container_path: Option<&str>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(container_path) = container_path {
        if let Some(parent) = Path::new(container_path).parent() {
            return parent.join(path);
        }
    }

    path.to_path_buf()
}

/// Work item for the reverb worker thread.
pub struct ReverbJob {
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
}

/// Result returned from the reverb worker thread.
pub struct ReverbResult {
    samples: Vec<f32>,
}

/// Spawn a worker thread to process convolution reverb jobs.
pub fn spawn_reverb_worker(
    mut reverb: Reverb,
    reverb_settings: Arc<Mutex<ReverbSettings>>,
    reverb_metrics: Arc<Mutex<ReverbMetrics>>,
) -> (mpsc::SyncSender<ReverbJob>, mpsc::Receiver<ReverbResult>) {
    let (job_sender, job_receiver) = mpsc::sync_channel::<ReverbJob>(1);
    let (result_sender, result_receiver) = mpsc::sync_channel::<ReverbResult>(1);

    thread::spawn(move || {
        let mut current_dry_wet = 0.000001_f32;
        #[cfg(feature = "debug")]
        let mut last_log = Instant::now();
        #[cfg(feature = "debug")]
        let mut avg_dsp_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_audio_ms = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut avg_rt_factor = 0.0_f64;
        #[cfg(feature = "debug")]
        let mut min_rt_factor = f64::INFINITY;
        #[cfg(feature = "debug")]
        let mut max_rt_factor = 0.0_f64;
        #[cfg(feature = "debug")]
        let alpha = 0.1_f64;

        #[cfg(not(feature = "debug"))]
        let _ = &reverb_metrics;

        while let Ok(job) = job_receiver.recv() {
            let settings = *reverb_settings.lock().unwrap();
            if (settings.dry_wet - current_dry_wet).abs() > f32::EPSILON {
                reverb.set_dry_wet(settings.dry_wet);
                current_dry_wet = settings.dry_wet;
            }

            #[cfg(feature = "debug")]
            let audio_time_ms = if job.channels > 0 && job.sample_rate > 0 {
                let frames = job.samples.len() as f64 / job.channels as f64;
                (frames / job.sample_rate as f64) * 1000.0
            } else {
                0.0
            };

            #[cfg(feature = "debug")]
            let dsp_start = Instant::now();
            let samples = if settings.enabled && settings.dry_wet > 0.0 {
                reverb.process(&job.samples)
            } else {
                job.samples
            };
            #[cfg(feature = "debug")]
            let dsp_time_ms = dsp_start.elapsed().as_secs_f64() * 1000.0;
            #[cfg(feature = "debug")]
            let rt_factor = if audio_time_ms > 0.0 {
                dsp_time_ms / audio_time_ms
            } else {
                0.0
            };

            #[cfg(feature = "debug")]
            {
                avg_dsp_ms = if avg_dsp_ms == 0.0 {
                    dsp_time_ms
                } else {
                    (avg_dsp_ms * (1.0 - alpha)) + (dsp_time_ms * alpha)
                };
                avg_audio_ms = if avg_audio_ms == 0.0 {
                    audio_time_ms
                } else {
                    (avg_audio_ms * (1.0 - alpha)) + (audio_time_ms * alpha)
                };
                avg_rt_factor = if avg_rt_factor == 0.0 {
                    rt_factor
                } else {
                    (avg_rt_factor * (1.0 - alpha)) + (rt_factor * alpha)
                };

                if rt_factor > 0.0 {
                    if rt_factor < min_rt_factor {
                        min_rt_factor = rt_factor;
                    }
                    if rt_factor > max_rt_factor {
                        max_rt_factor = rt_factor;
                    }
                }

                let mut metrics = reverb_metrics.lock().unwrap();
                metrics.dsp_time_ms = dsp_time_ms;
                metrics.audio_time_ms = audio_time_ms;
                metrics.rt_factor = rt_factor;
                metrics.avg_dsp_ms = avg_dsp_ms;
                metrics.avg_audio_ms = avg_audio_ms;
                metrics.avg_rt_factor = avg_rt_factor;
                metrics.min_rt_factor = if min_rt_factor.is_finite() {
                    min_rt_factor
                } else {
                    0.0
                };
                metrics.max_rt_factor = max_rt_factor;
            }

            #[cfg(feature = "debug")]
            if last_log.elapsed().as_secs_f64() >= 1.0 {
                info!(
                    "DSP packet: {:.2}ms / {:.2}ms ({:.2}x realtime) avg {:.2}x (min {:.2}x max {:.2}x)",
                    dsp_time_ms,
                    audio_time_ms,
                    rt_factor,
                    avg_rt_factor,
                    if min_rt_factor.is_finite() {
                        min_rt_factor
                    } else {
                        0.0
                    },
                    max_rt_factor
                );
                last_log = Instant::now();
            }

            if result_sender.send(ReverbResult { samples }).is_err() {
                break;
            }
        }
    });

    (job_sender, result_receiver)
}

/// Submit a reverb job and wait for the processed samples.
pub fn process_reverb(
    sender: &mpsc::SyncSender<ReverbJob>,
    receiver: &mpsc::Receiver<ReverbResult>,
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
) -> Vec<f32> {
    if sender
        .send(ReverbJob {
            samples,
            channels,
            sample_rate,
        })
        .is_err()
    {
        return Vec::new();
    }

    match receiver.recv() {
        Ok(result) => result.samples,
        Err(_) => Vec::new(),
    }
}
