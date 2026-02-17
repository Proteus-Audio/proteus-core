use rodio::buffer::SamplesBuffer;
use rodio::{OutputStreamBuilder, Sink};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::RecvTimeoutError, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, warn};

use crate::playback::engine::PlayerEngine;
use crate::tools::timer;

use super::{Player, PlayerState, OUTPUT_STREAM_OPEN_RETRIES, OUTPUT_STREAM_OPEN_RETRY_MS};

struct PlaybackThreadGuard {
    exists: Arc<AtomicBool>,
}

impl PlaybackThreadGuard {
    fn new(exists: Arc<AtomicBool>) -> Self {
        exists.store(true, Ordering::Relaxed);
        Self { exists }
    }
}

impl Drop for PlaybackThreadGuard {
    fn drop(&mut self) {
        self.exists.store(false, Ordering::Relaxed);
    }
}

impl Player {
    pub(super) fn initialize_thread(&mut self, ts: Option<f64>) {
        let mut finished_tracks = self.finished_tracks.lock().unwrap();
        finished_tracks.clear();
        drop(finished_tracks);

        self.abort = Arc::new(AtomicBool::new(false));
        self.playback_thread_exists.store(true, Ordering::SeqCst);
        let playback_id = self.playback_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.buffering_done.store(false, Ordering::SeqCst);
        let now_ms_value = now_ms();
        self.last_chunk_ms.store(now_ms_value, Ordering::Relaxed);
        self.last_time_update_ms
            .store(now_ms_value, Ordering::Relaxed);

        let play_state = self.state.clone();
        let abort = self.abort.clone();
        let playback_thread_exists = self.playback_thread_exists.clone();
        let playback_id_atomic = self.playback_id.clone();
        let time_passed = self.ts.clone();

        let duration = self.duration.clone();
        let prot = self.prot.clone();
        let buffer_settings = self.buffer_settings.clone();
        let buffer_settings_for_state = self.buffer_settings.clone();
        let effects = self.effects.clone();
        let inline_effects_update = self.inline_effects_update.clone();
        let dsp_metrics = self.dsp_metrics.clone();
        let dsp_metrics_for_sink = self.dsp_metrics.clone();
        let effects_reset = self.effects_reset.clone();
        let output_meter = self.output_meter.clone();
        let audio_info = self.info.clone();
        let next_resume_fade_ms = self.next_resume_fade_ms.clone();

        let audio_heard = self.audio_heard.clone();
        let volume = self.volume.clone();
        let sink_mutex = self.sink.clone();
        let buffer_done_thread_flag = self.buffering_done.clone();
        let last_chunk_ms = self.last_chunk_ms.clone();
        let last_time_update_ms = self.last_time_update_ms.clone();

        audio_heard.store(false, Ordering::Relaxed);

        {
            let mut meter = self.output_meter.lock().unwrap();
            meter.reset();
        }

        thread::spawn(move || {
            let thread_guard = PlaybackThreadGuard::new(playback_thread_exists.clone());

            let start_time = ts.unwrap_or(0.0);
            let mut engine = PlayerEngine::new(
                prot,
                Some(abort.clone()),
                start_time,
                buffer_settings,
                effects,
                dsp_metrics,
                effects_reset,
                inline_effects_update,
            );
            let mut stream = None;
            for attempt in 1..=OUTPUT_STREAM_OPEN_RETRIES {
                match OutputStreamBuilder::open_default_stream() {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(err) => {
                        if attempt == OUTPUT_STREAM_OPEN_RETRIES {
                            error!(
                                "failed to open default output stream after {} attempts: {}",
                                OUTPUT_STREAM_OPEN_RETRIES, err
                            );
                            return;
                        }
                        warn!(
                            "open_default_stream attempt {}/{} failed: {}",
                            attempt, OUTPUT_STREAM_OPEN_RETRIES, err
                        );
                        thread::sleep(Duration::from_millis(OUTPUT_STREAM_OPEN_RETRY_MS));
                    }
                }
            }
            let stream = stream.expect("stream should exist after successful retry loop");
            let mixer = stream.mixer().clone();

            let mut sink = sink_mutex.lock().unwrap();
            *sink = Sink::connect_new(&mixer);
            sink.pause();
            sink.set_volume(*volume.lock().unwrap());
            drop(sink);

            let mut duration = duration.lock().unwrap();
            *duration = engine.get_duration();
            drop(duration);

            let chunk_lengths = Arc::new(Mutex::new(Vec::new()));
            let mut time_passed_unlocked = time_passed.lock().unwrap();
            *time_passed_unlocked = start_time;
            drop(time_passed_unlocked);

            let pause_sink = |sink: &Sink, fade_length_out_seconds: f32| {
                let timestamp = *time_passed.lock().unwrap();

                let fade_increments = sink.volume() / (fade_length_out_seconds * 100.0);
                while sink.volume() > 0.0 && timestamp != start_time {
                    sink.set_volume(sink.volume() - fade_increments);
                    thread::sleep(Duration::from_millis(10));
                }
                sink.pause();
            };

            let resume_sink = |sink: &Sink, fade_length_in_seconds: f32| {
                let volume = *volume.lock().unwrap();
                if fade_length_in_seconds <= 0.0 {
                    sink.play();
                    sink.set_volume(volume);
                    return;
                }
                let fade_increments = (volume - sink.volume()) / (fade_length_in_seconds * 100.0);
                sink.play();
                while sink.volume() < volume {
                    sink.set_volume(sink.volume() + fade_increments);
                    thread::sleep(Duration::from_millis(5));
                }
            };

            {
                let startup_settings = buffer_settings_for_state.lock().unwrap();
                let startup_silence_ms = startup_settings.startup_silence_ms;
                drop(startup_settings);

                let sample_rate = audio_info.sample_rate as u32;
                let channels = audio_info.channels as u16;

                if startup_silence_ms > 0.0 {
                    let samples = ((startup_silence_ms / 1000.0) * sample_rate as f32).ceil()
                        as usize
                        * channels as usize;
                    let silence = vec![0.0_f32; samples.max(1)];
                    let silence_buffer = SamplesBuffer::new(channels, sample_rate, silence);
                    let sink = sink_mutex.lock().unwrap();
                    sink.append(silence_buffer);
                    drop(sink);
                }
            }

            let startup_fade_pending = Cell::new(true);
            let check_details = || {
                if abort.load(Ordering::SeqCst) {
                    let sink = sink_mutex.lock().unwrap();
                    pause_sink(&sink, 0.1);
                    sink.clear();
                    drop(sink);

                    return false;
                }

                let sink = sink_mutex.lock().unwrap();
                let state = play_state.lock().unwrap().clone();
                let start_sink_chunks = buffer_settings_for_state.lock().unwrap().start_sink_chunks;
                if state == PlayerState::Resuming
                    && start_sink_chunks > 0
                    && sink.len() < start_sink_chunks
                {
                    sink.pause();
                    drop(sink);
                    return true;
                }
                if state == PlayerState::Pausing {
                    pause_sink(&sink, 0.1);
                    play_state.lock().unwrap().clone_from(&PlayerState::Paused);
                }
                if state == PlayerState::Resuming {
                    let fade_length = if startup_fade_pending.replace(false) {
                        if let Some(ms) = next_resume_fade_ms.lock().unwrap().take() {
                            (ms / 1000.0).max(0.0)
                        } else {
                            let startup_fade_ms =
                                buffer_settings_for_state.lock().unwrap().startup_fade_ms;
                            (startup_fade_ms / 1000.0).max(0.0)
                        }
                    } else {
                        0.1
                    };
                    resume_sink(&sink, fade_length);
                    play_state.lock().unwrap().clone_from(&PlayerState::Playing);
                }
                drop(sink);

                true
            };

            let time_chunks_mutex = Arc::new(Mutex::new(start_time));
            let timer_mut = Arc::new(Mutex::new(timer::Timer::new()));
            let buffering_done = Arc::new(AtomicBool::new(false));
            let buffering_done_flag = buffer_done_thread_flag.clone();
            let final_duration = Arc::new(Mutex::new(None::<f64>));
            let mut timer = timer_mut.lock().unwrap();
            timer.start();
            drop(timer);

            let last_meter_time = Cell::new(0.0_f64);
            let update_chunk_lengths = || {
                if abort.load(Ordering::SeqCst) {
                    return;
                }

                let mut chunk_lengths = chunk_lengths.lock().unwrap();
                let mut time_passed_unlocked = time_passed.lock().unwrap();
                let mut time_chunks_passed = time_chunks_mutex.lock().unwrap();
                let mut timer = timer_mut.lock().unwrap();
                last_time_update_ms.store(now_ms(), Ordering::Relaxed);
                let sink = sink_mutex.lock().unwrap();
                if !buffering_done.load(Ordering::Relaxed) {
                    let chunks_played = chunk_lengths.len().saturating_sub(sink.len());

                    for _ in 0..chunks_played {
                        timer.reset();
                        timer.start();
                        *time_chunks_passed += chunk_lengths.remove(0);
                    }
                }

                if sink.is_paused() {
                    timer.pause();
                } else {
                    timer.un_pause();
                }

                let current_audio_time = *time_chunks_passed + timer.get_time().as_secs_f64();
                let delta = (current_audio_time - last_meter_time.get()).max(0.0);
                last_meter_time.set(current_audio_time);
                {
                    let mut meter = output_meter.lock().unwrap();
                    meter.advance(delta);
                }

                *time_passed_unlocked = current_audio_time;

                drop(sink);
                drop(chunk_lengths);
                drop(time_passed_unlocked);
                drop(time_chunks_passed);
                drop(timer);
            };

            let append_timing = Arc::new(Mutex::new((Instant::now(), 0.0_f64, 0_u64, 0.0_f64)));
            let update_sink = |(mixer, length_in_seconds): (SamplesBuffer, f64)| {
                if playback_id_atomic.load(Ordering::SeqCst) != playback_id {
                    return;
                }
                let max_sink_chunks = {
                    let settings = buffer_settings_for_state.lock().unwrap();
                    settings.max_sink_chunks
                };
                if max_sink_chunks > 0 {
                    loop {
                        if abort.load(Ordering::SeqCst) {
                            return;
                        }
                        if playback_id_atomic.load(Ordering::SeqCst) != playback_id {
                            return;
                        }
                        let sink_len = {
                            let sink = sink_mutex.lock().unwrap();
                            sink.len()
                        };
                        if sink_len < max_sink_chunks {
                            break;
                        }
                        update_chunk_lengths();
                        if !check_details() {
                            return;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                let (delay_ms, late) = {
                    let mut timing = append_timing.lock().unwrap();
                    let now = Instant::now();
                    let delta_ms = now.duration_since(timing.0).as_secs_f64() * 1000.0;
                    let chunk_ms = length_in_seconds * 1000.0;
                    let late = delta_ms > (chunk_ms * 1.2) && chunk_ms > 0.0;
                    if late {
                        timing.2 = timing.2.saturating_add(1);
                    }
                    timing.1 = if timing.1 == 0.0 {
                        delta_ms
                    } else {
                        (timing.1 * 0.9) + (delta_ms * 0.1)
                    };
                    timing.3 = timing.3.max(delta_ms);
                    timing.0 = now;
                    (delta_ms, late)
                };
                audio_heard.store(true, Ordering::Relaxed);
                last_chunk_ms.store(now_ms(), Ordering::Relaxed);

                {
                    let mut meter = output_meter.lock().unwrap();
                    meter.push_samples(&mixer);
                }
                {
                    let mut metrics = dsp_metrics_for_sink.lock().unwrap();
                    metrics.append_delay_ms = delay_ms;
                    metrics.avg_append_delay_ms = {
                        if metrics.avg_append_delay_ms == 0.0 {
                            delay_ms
                        } else {
                            (metrics.avg_append_delay_ms * 0.9) + (delay_ms * 0.1)
                        }
                    };
                    metrics.max_append_delay_ms = metrics.max_append_delay_ms.max(delay_ms);
                    metrics.late_append_count = {
                        let timing = append_timing.lock().unwrap();
                        timing.2
                    };
                    metrics.late_append_active = late;
                }

                let sink = sink_mutex.lock().unwrap();
                let append_jitter_log_ms = {
                    let settings = buffer_settings_for_state.lock().unwrap();
                    settings.append_jitter_log_ms
                };
                if append_jitter_log_ms > 0.0 && (late || delay_ms > append_jitter_log_ms as f64) {
                    let expected_ms = length_in_seconds * 1000.0;
                    log::info!(
                        "append jitter: delta={:.2}ms expected={:.2}ms late={} threshold={:.2}ms sink_len={}",
                        delay_ms,
                        expected_ms,
                        late,
                        append_jitter_log_ms,
                        sink.len()
                    );
                }
                let mut chunk_lengths = chunk_lengths.lock().unwrap();

                sink.append(mixer);

                drop(sink);

                chunk_lengths.push(length_in_seconds);
                drop(chunk_lengths);

                update_chunk_lengths();
                check_details();
            };

            let receiver = engine.start_receiver();
            loop {
                match receiver.recv_timeout(Duration::from_millis(20)) {
                    Ok(chunk) => {
                        update_sink(chunk);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        update_chunk_lengths();
                        if !check_details() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            #[cfg(feature = "debug")]
            log::info!("engine reception loop finished");

            buffering_done.store(true, Ordering::Relaxed);
            buffering_done_flag.store(true, Ordering::Relaxed);
            {
                let mut final_duration = final_duration.lock().unwrap();
                if final_duration.is_none() {
                    let chunk_lengths = chunk_lengths.lock().unwrap();
                    let time_chunks_passed = time_chunks_mutex.lock().unwrap();
                    *final_duration = Some(*time_chunks_passed + chunk_lengths.iter().sum::<f64>());
                }
            }

            #[cfg(feature = "debug")]
            {
                let sink = sink_mutex.lock().unwrap();
                let paused = sink.is_paused();
                let empty = sink.empty();
                let sink_len = sink.len();
                drop(sink);
                let time_passed = *time_passed.lock().unwrap();
                let final_duration = *final_duration.lock().unwrap();
                log::info!(
                    "Starting drain loop: paused={} empty={} sink_len={} time={:.3} final={:?}",
                    paused,
                    empty,
                    sink_len,
                    time_passed,
                    final_duration
                );
            }

            loop {
                update_chunk_lengths();
                if !check_details() {
                    break;
                }

                let done = if engine.finished_buffering() {
                    if let Some(final_duration) = *final_duration.lock().unwrap() {
                        let time_passed = *time_passed.lock().unwrap();
                        time_passed >= (final_duration - 0.001).max(0.0)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if done {
                    break;
                }

                thread::sleep(Duration::from_millis(10));
            }

            #[cfg(feature = "debug")]
            log::info!("Finished drain loop!");

            drop(thread_guard);
        });
    }
}

fn now_ms() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
