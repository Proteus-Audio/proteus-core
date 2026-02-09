//! High-level playback controller for the Proteus library.

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStreamBuilder, Sink};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc::RecvTimeoutError, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::audio::samples::clone_samples_buffer;
use crate::container::prot::Prot;
use crate::diagnostics::reporter::{Report, Reporter};
use crate::dsp::effects::convolution_reverb::{parse_impulse_response_string, ImpulseResponseSpec};
use crate::playback::output_meter::OutputMeter;
use crate::tools::timer;
use crate::{
    container::info::Info,
    dsp::effects::AudioEffect,
    playback::engine::{DspChainMetrics, PlaybackBufferSettings, PlayerEngine},
};

/// High-level playback state for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Init,
    Resuming,
    Playing,
    Pausing,
    Paused,
    Stopping,
    Stopped,
    Finished,
}

/// Snapshot of convolution reverb settings for UI consumers.
#[derive(Debug, Clone, Copy)]
pub struct ReverbSettingsSnapshot {
    pub enabled: bool,
    pub dry_wet: f32,
}

const OUTPUT_METER_REFRESH_HZ: f32 = 30.0;

/// Primary playback controller.
///
/// `Player` owns the playback threads, buffering state, and runtime settings
/// such as volume and reverb configuration.
#[derive(Clone)]
pub struct Player {
    pub info: Info,
    pub finished_tracks: Arc<Mutex<Vec<i32>>>,
    pub ts: Arc<Mutex<f64>>,
    state: Arc<Mutex<PlayerState>>,
    abort: Arc<AtomicBool>,
    playback_thread_exists: Arc<AtomicBool>,
    playback_id: Arc<AtomicU64>,
    duration: Arc<Mutex<f64>>,
    prot: Arc<Mutex<Prot>>,
    audio_heard: Arc<AtomicBool>,
    volume: Arc<Mutex<f32>>,
    sink: Arc<Mutex<Sink>>,
    audition_source: Arc<Mutex<Option<SamplesBuffer>>>,
    reporter: Option<Arc<Mutex<Reporter>>>,
    buffer_settings: Arc<Mutex<PlaybackBufferSettings>>,
    effects: Arc<Mutex<Vec<AudioEffect>>>,
    dsp_metrics: Arc<Mutex<DspChainMetrics>>,
    effects_reset: Arc<AtomicU64>,
    output_meter: Arc<Mutex<OutputMeter>>,
    buffering_done: Arc<AtomicBool>,
    last_chunk_ms: Arc<AtomicU64>,
    last_time_update_ms: Arc<AtomicU64>,
    impulse_response_override: Option<ImpulseResponseSpec>,
    impulse_response_tail_override: Option<f32>,
}

impl Player {
    /// Create a new player for a single container path.
    pub fn new(file_path: &String) -> Self {
        let this = Self::new_from_path_or_paths(Some(file_path), None);
        this
    }

    /// Create a new player for a set of standalone file paths.
    pub fn new_from_file_paths(file_paths: &Vec<Vec<String>>) -> Self {
        let this = Self::new_from_path_or_paths(None, Some(file_paths));
        this
    }

    /// Create a player from either a container path or standalone file paths.
    pub fn new_from_path_or_paths(path: Option<&String>, paths: Option<&Vec<Vec<String>>>) -> Self {
        let (prot, info) = match path {
            Some(path) => {
                let prot = Arc::new(Mutex::new(Prot::new(path)));
                let info = Info::new(path.clone());
                (prot, info)
            }
            None => {
                let prot = Arc::new(Mutex::new(Prot::new_from_file_paths(paths.unwrap())));
                let locked_prot = prot.lock().unwrap();
                let info = Info::new_from_file_paths(locked_prot.get_file_paths_dictionary());
                drop(locked_prot);
                (prot, info)
            }
        };

        let (sink, _queue) = Sink::new();
        let sink: Arc<Mutex<Sink>> = Arc::new(Mutex::new(sink));

        let channels = info.channels as usize;
        let sample_rate = info.sample_rate;
        let effects = {
            let prot_locked = prot.lock().unwrap();
            match prot_locked.get_effects() {
                Some(effects) => Arc::new(Mutex::new(effects)),
                None => Arc::new(Mutex::new(vec![])),
            }
        };

        let mut this = Self {
            info,
            finished_tracks: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(PlayerState::Stopped)),
            abort: Arc::new(AtomicBool::new(false)),
            ts: Arc::new(Mutex::new(0.0)),
            playback_thread_exists: Arc::new(AtomicBool::new(true)),
            playback_id: Arc::new(AtomicU64::new(0)),
            duration: Arc::new(Mutex::new(0.0)),
            audio_heard: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(Mutex::new(0.8)),
            sink,
            audition_source: Arc::new(Mutex::new(None)),
            prot,
            reporter: None,
            buffer_settings: Arc::new(Mutex::new(PlaybackBufferSettings::new(20.0))),
            effects,
            dsp_metrics: Arc::new(Mutex::new(DspChainMetrics::default())),
            effects_reset: Arc::new(AtomicU64::new(0)),
            output_meter: Arc::new(Mutex::new(OutputMeter::new(
                channels,
                sample_rate,
                OUTPUT_METER_REFRESH_HZ,
            ))),
            buffering_done: Arc::new(AtomicBool::new(false)),
            last_chunk_ms: Arc::new(AtomicU64::new(0)),
            last_time_update_ms: Arc::new(AtomicU64::new(0)),
            impulse_response_override: None,
            impulse_response_tail_override: None,
        };

        this.initialize_thread(None);

        this
    }

    /// Override the impulse response used for convolution reverb.
    pub fn set_impulse_response_spec(&mut self, spec: ImpulseResponseSpec) {
        self.impulse_response_override = Some(spec.clone());
        let mut prot = self.prot.lock().unwrap();
        prot.set_impulse_response_spec(spec);
        self.request_effects_reset();
    }

    /// Parse and apply an impulse response spec string.
    pub fn set_impulse_response_from_string(&mut self, value: &str) {
        if let Some(spec) = parse_impulse_response_string(value) {
            self.set_impulse_response_spec(spec);
        }
    }

    /// Override the impulse response tail trim (dB).
    pub fn set_impulse_response_tail_db(&mut self, tail_db: f32) {
        self.impulse_response_tail_override = Some(tail_db);
        let mut prot = self.prot.lock().unwrap();
        prot.set_impulse_response_tail_db(tail_db);
        self.request_effects_reset();
    }

    /// Enable or disable convolution reverb.
    pub fn set_reverb_enabled(&self, enabled: bool) {
        let mut effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_convolution_reverb_mut())
        {
            effect.enabled = enabled;
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_delay_reverb_mut())
        {
            effect.enabled = enabled;
        }
    }

    /// Set the reverb wet/dry mix (clamped to `[0.0, 1.0]`).
    pub fn set_reverb_mix(&self, dry_wet: f32) {
        let mut effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_convolution_reverb_mut())
        {
            effect.dry_wet = dry_wet.clamp(0.0, 1.0);
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_delay_reverb_mut())
        {
            effect.mix = dry_wet.clamp(0.0, 1.0);
        }
        if let Some(effect) = effects
            .iter_mut()
            .find_map(|effect| effect.as_diffusion_reverb_mut())
        {
            effect.mix = dry_wet.clamp(0.0, 1.0);
        }
    }

    /// Retrieve the current reverb settings snapshot.
    pub fn get_reverb_settings(&self) -> ReverbSettingsSnapshot {
        let effects = self.effects.lock().unwrap();
        if let Some(effect) = effects
            .iter()
            .find_map(|effect| effect.as_convolution_reverb())
        {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.dry_wet,
            };
        }
        if let Some(effect) = effects
            .iter()
            .find_map(|effect| effect.as_diffusion_reverb())
        {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.mix,
            };
        }
        if let Some(effect) = effects.iter().find_map(|effect| effect.as_delay_reverb()) {
            return ReverbSettingsSnapshot {
                enabled: effect.enabled,
                dry_wet: effect.mix,
            };
        }
        ReverbSettingsSnapshot {
            enabled: false,
            dry_wet: 0.0,
        }
    }

    /// Snapshot the active effect chain names.
    pub fn get_effect_names(&self) -> Vec<String> {
        let effects = self.effects.lock().unwrap();
        effects
            .iter()
            .map(|effect| match effect {
                AudioEffect::DelayReverb(_) => "DelayReverb".to_string(),
                AudioEffect::BasicReverb(_) => "DelayReverb".to_string(),
                AudioEffect::DiffusionReverb(_) => "DiffusionReverb".to_string(),
                AudioEffect::ConvolutionReverb(_) => "ConvolutionReverb".to_string(),
                AudioEffect::LowPassFilter(_) => "LowPassFilter".to_string(),
                AudioEffect::HighPassFilter(_) => "HighPassFilter".to_string(),
                AudioEffect::Distortion(_) => "Distortion".to_string(),
                AudioEffect::Compressor(_) => "Compressor".to_string(),
                AudioEffect::Limiter(_) => "Limiter".to_string(),
            })
            .collect()
    }

    /// Replace the active DSP effects chain.
    pub fn set_effects(&mut self, effects: Vec<AudioEffect>) {
        {
            let mut guard = self.effects.lock().unwrap();
            println!("New Effects: {:?}", effects);
            *guard = effects;
        }
        self.request_effects_reset();

        // Seeking to the current time stamp refreshes the
        // Sink so that the new effects are applied immediately.
        if !self.thread_finished() {
            let ts = self.get_time();
            self.seek(ts);
        }
    }

    /// Retrieve the latest DSP chain performance metrics.
    pub fn get_dsp_metrics(&self) -> DspChainMetrics {
        *self.dsp_metrics.lock().unwrap()
    }

    /// Retrieve the most recent per-channel peak levels.
    pub fn get_levels(&self) -> Vec<f32> {
        self.output_meter.lock().unwrap().levels()
    }

    /// Retrieve the most recent per-channel peak levels in dBFS.
    pub fn get_levels_db(&self) -> Vec<f32> {
        self.output_meter
            .lock()
            .unwrap()
            .levels()
            .into_iter()
            .map(linear_to_dbfs)
            .collect()
    }

    /// Retrieve the most recent per-channel average levels.
    pub fn get_levels_avg(&self) -> Vec<f32> {
        self.output_meter.lock().unwrap().averages()
    }

    /// Set the output meter refresh rate (frames per second).
    pub fn set_output_meter_refresh_hz(&self, hz: f32) {
        self.output_meter.lock().unwrap().set_refresh_hz(hz);
    }

    /// Debug helper returning thread alive, state, and audio heard flags.
    pub fn debug_playback_state(&self) -> (bool, PlayerState, bool) {
        (
            self.playback_thread_exists.load(Ordering::SeqCst),
            *self.state.lock().unwrap(),
            self.audio_heard.load(Ordering::Relaxed),
        )
    }

    /// Debug helper indicating whether buffering has completed.
    pub fn debug_buffering_done(&self) -> bool {
        self.buffering_done.load(Ordering::Relaxed)
    }

    /// Debug helper returning internal timing markers in milliseconds.
    pub fn debug_timing_ms(&self) -> (u64, u64) {
        (
            self.last_chunk_ms.load(Ordering::Relaxed),
            self.last_time_update_ms.load(Ordering::Relaxed),
        )
    }

    /// Debug helper returning sink paused/empty flags and queued length.
    pub fn debug_sink_state(&self) -> (bool, bool, usize) {
        let sink = self.sink.lock().unwrap();
        let paused = sink.is_paused();
        let empty = sink.empty();
        let len = sink.len();
        (paused, empty, len)
    }

    fn request_effects_reset(&self) {
        self.effects_reset.fetch_add(1, Ordering::SeqCst);
    }

    /// Configure the minimum buffered audio (ms) before playback starts.
    pub fn set_start_buffer_ms(&self, start_buffer_ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.start_buffer_ms = start_buffer_ms.max(0.0);
    }

    /// Configure heuristic end-of-track threshold for containers (ms).
    pub fn set_track_eos_ms(&self, track_eos_ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.track_eos_ms = track_eos_ms.max(0.0);
    }

    /// Configure minimum sink chunks queued before playback starts/resumes.
    pub fn set_start_sink_chunks(&self, chunks: usize) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.start_sink_chunks = chunks;
    }

    /// Configure the startup silence pre-roll (ms).
    pub fn set_startup_silence_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.startup_silence_ms = ms.max(0.0);
    }

    /// Configure the startup fade-in length (ms).
    pub fn set_startup_fade_ms(&self, ms: f32) {
        let mut settings = self.buffer_settings.lock().unwrap();
        settings.startup_fade_ms = ms.max(0.0);
    }

    fn audition(&self, length: Duration) {
        let audition_source_mutex = self.audition_source.clone();

        // Create new thread to audition
        thread::spawn(move || {
            // Wait until audition source is ready
            while audition_source_mutex.lock().unwrap().is_none() {
                thread::sleep(Duration::from_millis(10));
            }

            let audition_source_option = audition_source_mutex.lock().unwrap().take();
            let audition_source = audition_source_option.unwrap();

            let _audition_stream = OutputStreamBuilder::open_default_stream().unwrap();
            let audition_sink = Sink::connect_new(_audition_stream.mixer());
            audition_sink.pause();
            audition_sink.set_volume(0.8);
            audition_sink.append(audition_source);
            audition_sink.play();
            thread::sleep(length);
            audition_sink.pause();
        });
    }

    fn initialize_thread(&mut self, ts: Option<f64>) {
        // Empty finished_tracks
        let mut finished_tracks = self.finished_tracks.lock().unwrap();
        finished_tracks.clear();
        drop(finished_tracks);

        // ===== Set play options ===== //
        self.abort.store(false, Ordering::SeqCst);
        self.playback_thread_exists.store(true, Ordering::SeqCst);
        let playback_id = self.playback_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.buffering_done.store(false, Ordering::SeqCst);
        let now_ms_value = now_ms();
        self.last_chunk_ms.store(now_ms_value, Ordering::Relaxed);
        self.last_time_update_ms
            .store(now_ms_value, Ordering::Relaxed);

        // ===== Clone variables ===== //
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
        let dsp_metrics = self.dsp_metrics.clone();
        let effects_reset = self.effects_reset.clone();
        let output_meter = self.output_meter.clone();
        let audio_info = self.info.clone();

        let audio_heard = self.audio_heard.clone();
        let volume = self.volume.clone();
        let sink_mutex = self.sink.clone();
        let audition_source_mutex = self.audition_source.clone();
        let buffer_done_thread_flag = self.buffering_done.clone();
        let last_chunk_ms = self.last_chunk_ms.clone();
        let last_time_update_ms = self.last_time_update_ms.clone();

        audio_heard.store(false, Ordering::Relaxed);

        // clear audition source
        let mut audition_source = audition_source_mutex.lock().unwrap();
        *audition_source = None;
        drop(audition_source);

        {
            let mut meter = self.output_meter.lock().unwrap();
            meter.reset();
        }

        // ===== Start playback ===== //
        thread::spawn(move || {
            // ===================== //
            // Set playback_thread_exists to true
            // ===================== //
            playback_thread_exists.store(true, Ordering::Relaxed);

            // ===================== //
            // Initialize engine & sink
            // ===================== //
            let start_time = match ts {
                Some(ts) => ts,
                None => 0.0,
            };
            let mut engine = PlayerEngine::new(
                prot,
                Some(abort.clone()),
                start_time,
                buffer_settings,
                effects,
                dsp_metrics,
                effects_reset,
            );
            let _stream = OutputStreamBuilder::open_default_stream().unwrap();
            let mixer = _stream.mixer().clone();

            let mut sink = sink_mutex.lock().unwrap();
            *sink = Sink::connect_new(&mixer);
            sink.pause();
            sink.set_volume(*volume.lock().unwrap());
            drop(sink);

            // ===================== //
            // Set duration from engine
            // ===================== //
            let mut duration = duration.lock().unwrap();
            *duration = engine.get_duration();
            drop(duration);

            // ===================== //
            // Initialize chunk_lengths & time_passed
            // ===================== //
            let chunk_lengths = Arc::new(Mutex::new(Vec::new()));
            let mut time_passed_unlocked = time_passed.lock().unwrap();
            *time_passed_unlocked = start_time;
            drop(time_passed_unlocked);

            let pause_sink = |sink: &Sink, fade_length_out_seconds: f32| {
                let timestamp = *time_passed.lock().unwrap();

                let fade_increments = sink.volume() / (fade_length_out_seconds * 100.0);
                // Fade out and pause sink
                while sink.volume() > 0.0 && timestamp != start_time {
                    sink.set_volume(sink.volume() - fade_increments);
                    thread::sleep(Duration::from_millis(10));
                }
                sink.pause();
            };

            let resume_sink = |sink: &Sink, fade_length_in_seconds: f32| {
                let volume = *volume.lock().unwrap();
                let fade_increments = (volume - sink.volume()) / (fade_length_in_seconds * 100.0);
                // Fade in and play sink
                sink.play();
                while sink.volume() < volume {
                    sink.set_volume(sink.volume() + fade_increments);
                    thread::sleep(Duration::from_millis(5));
                }
            };

            // ===================== //
            // Start sink with startup silence + fade in
            // ===================== //
            {
                let startup_settings = buffer_settings_for_state.lock().unwrap();
                let startup_silence_ms = startup_settings.startup_silence_ms;
                let startup_fade_ms = startup_settings.startup_fade_ms;
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

                if startup_fade_ms > 0.0 {
                    resume_sink(
                        &sink_mutex.lock().unwrap(),
                        (startup_fade_ms / 1000.0).max(0.0),
                    );
                }
            }

            // ===================== //
            // Check if the player should be paused or not
            // ===================== //
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
                    // Keep paused until enough chunks are queued.
                    sink.pause();
                    drop(sink);
                    return true;
                }
                if state == PlayerState::Pausing {
                    pause_sink(&sink, 0.1);
                    play_state.lock().unwrap().clone_from(&PlayerState::Paused);
                }
                if state == PlayerState::Resuming {
                    resume_sink(&sink, 0.1);
                    play_state.lock().unwrap().clone_from(&PlayerState::Playing);
                }
                drop(sink);

                true
            };

            // ===================== //
            // Update chunk_lengths / time_passed
            // ===================== //
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
                    // Check how many chunks have been played (chunk_lengths.len() - sink.len())
                    // since the last time this function was called and add that to time_passed.
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

            // ===================== //
            // Update sink for each chunk received from engine
            // ===================== //
            let update_sink = |(mixer, length_in_seconds): (SamplesBuffer, f64)| {
                if playback_id_atomic.load(Ordering::SeqCst) != playback_id {
                    return;
                }
                audio_heard.store(true, Ordering::Relaxed);
                last_chunk_ms.store(now_ms(), Ordering::Relaxed);

                {
                    let mut meter = output_meter.lock().unwrap();
                    meter.push_samples(&mixer);
                }

                let mut audition_source = audition_source_mutex.lock().unwrap();
                let sink = sink_mutex.lock().unwrap();
                let mut chunk_lengths = chunk_lengths.lock().unwrap();

                let total_time = chunk_lengths.iter().sum::<f64>();

                // If total_time is less than 0.2 seconds, audition the chunk
                if audition_source.is_none() {
                    let (mixer_clone, mixer) = clone_samples_buffer(mixer);
                    *audition_source = Some(mixer_clone);
                    drop(audition_source);
                    sink.append(mixer);
                } else {
                    sink.append(mixer);
                }

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

            // From here on, all audio is buffered. Stop relying on sink.len()
            // to advance time so the UI keeps updating while the last buffer plays.
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

            // ===================== //
            // Wait until all tracks are finished playing in sink
            // ===================== //
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

            // ===================== //
            // Set playback_thread_exists to false
            // ===================== //
            playback_thread_exists.store(false, Ordering::Relaxed);
        });
    }

    /// Start playback from a specific timestamp (seconds).
    pub fn play_at(&mut self, ts: f64) {
        let mut timestamp = self.ts.lock().unwrap();
        *timestamp = ts;
        drop(timestamp);

        self.request_effects_reset();
        self.kill_current();
        // self.stop.store(false, Ordering::SeqCst);
        self.initialize_thread(Some(ts));

        self.resume();

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Start playback from the current timestamp.
    pub fn play(&mut self) {
        info!("Playing audio");
        let thread_exists = self.playback_thread_exists.load(Ordering::SeqCst);
        // self.stop.store(false, Ordering::SeqCst);

        if !thread_exists {
            self.initialize_thread(None);
        }

        self.resume();

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    /// Pause playback.
    pub fn pause(&self) {
        self.state.lock().unwrap().clone_from(&PlayerState::Pausing);
    }

    /// Resume playback if paused.
    pub fn resume(&self) {
        self.state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Resuming);
    }

    /// Stop the current playback thread without changing state.
    pub fn kill_current(&self) {
        self.state
            .lock()
            .unwrap()
            .clone_from(&PlayerState::Stopping);
        {
            let sink = self.sink.lock().unwrap();
            sink.stop();
        }
        self.abort.store(true, Ordering::SeqCst);

        while !self.thread_finished() {
            thread::sleep(Duration::from_millis(10));
        }

        self.state.lock().unwrap().clone_from(&PlayerState::Stopped);
    }

    /// Stop playback and reset timing state.
    pub fn stop(&self) {
        self.kill_current();
        self.ts.lock().unwrap().clone_from(&0.0);
    }

    /// Return true if playback is currently active.
    pub fn is_playing(&self) -> bool {
        let state = self.state.lock().unwrap();
        *state == PlayerState::Playing
    }

    /// Return true if playback is currently paused.
    pub fn is_paused(&self) -> bool {
        let state = self.state.lock().unwrap();
        *state == PlayerState::Paused
    }

    /// Get the current playback time in seconds.
    pub fn get_time(&self) -> f64 {
        let ts = self.ts.lock().unwrap();
        *ts
    }

    fn thread_finished(&self) -> bool {
        let playback_thread_exists = self.playback_thread_exists.load(Ordering::SeqCst);
        !playback_thread_exists
    }

    /// Return true if playback has reached the end.
    pub fn is_finished(&self) -> bool {
        self.thread_finished()
        // let state = self.state.lock().unwrap();
        // *state == PlayerState::Finished
    }

    /// Block the current thread until playback finishes.
    pub fn sleep_until_end(&self) {
        loop {
            if self.thread_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Get the total duration (seconds) of the active selection.
    pub fn get_duration(&self) -> f64 {
        let duration = self.duration.lock().unwrap();
        *duration
    }

    /// Seek to the given timestamp (seconds).
    pub fn seek(&mut self, ts: f64) {
        let mut timestamp = self.ts.lock().unwrap();
        *timestamp = ts;
        drop(timestamp);

        self.request_effects_reset();
        let state = self.state.lock().unwrap().clone();

        self.kill_current();
        self.state.lock().unwrap().clone_from(&state);
        self.initialize_thread(Some(ts));

        match state {
            PlayerState::Playing => self.resume(),
            PlayerState::Paused => {
                self.audition(Duration::from_millis(100));
            }
            _ => {}
        }
    }

    /// Refresh active track selections from the underlying container.
    pub fn refresh_tracks(&mut self) {
        let mut prot = self.prot.lock().unwrap();
        prot.refresh_tracks();
        if let Some(spec) = self.impulse_response_override.clone() {
            prot.set_impulse_response_spec(spec);
        }
        if let Some(tail_db) = self.impulse_response_tail_override {
            prot.set_impulse_response_tail_db(tail_db);
        }
        drop(prot);

        self.request_effects_reset();
        // If stopped, return
        if self.thread_finished() {
            return;
        }

        // Kill current thread and start
        // new thread at the current timestamp
        let ts = self.get_time();
        self.seek(ts);

        // If previously playing, resume
        if self.is_playing() {
            self.resume();
        }

        self.wait_for_audio_heard(Duration::from_secs(5));
    }

    fn wait_for_audio_heard(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if self.audio_heard.load(Ordering::Relaxed) {
                return true;
            }
            if self.thread_finished() {
                warn!("playback thread ended before audio was heard");
                return false;
            }
            if start.elapsed() >= timeout {
                warn!("timed out waiting for audio to start");
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Shuffle track selections and restart playback.
    pub fn shuffle(&mut self) {
        self.refresh_tracks();
    }

    /// Set the playback volume (0.0-1.0).
    pub fn set_volume(&mut self, new_volume: f32) {
        let sink = self.sink.lock().unwrap();
        sink.set_volume(new_volume);
        drop(sink);

        let mut volume = self.volume.lock().unwrap();
        *volume = new_volume;
        drop(volume);
    }

    /// Get the current playback volume.
    pub fn get_volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    /// Get the track identifiers used for display.
    pub fn get_ids(&self) -> Vec<String> {
        let prot = self.prot.lock().unwrap();

        return prot.get_ids();
    }

    /// Enable periodic reporting of playback status for UI consumers.
    pub fn set_reporting(
        &mut self,
        reporting: Arc<Mutex<dyn Fn(Report) + Send>>,
        reporting_interval: Duration,
    ) {
        if self.reporter.is_some() {
            self.reporter.as_ref().unwrap().lock().unwrap().stop();
        }

        let reporter = Arc::new(Mutex::new(Reporter::new(
            Arc::new(Mutex::new(self.clone())),
            reporting,
            reporting_interval,
        )));

        reporter.lock().unwrap().start();

        self.reporter = Some(reporter);
    }
}

fn linear_to_dbfs(value: f32) -> f32 {
    if value <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * value.log10()
    }
}

fn now_ms() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
