//! Runner for CLI execution, TUI lifecycle, and playback thread orchestration.

use std::{
    collections::VecDeque,
    fs,
    io,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use clap::ArgMatches;
use crossterm::{
    cursor, event, execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{error, info};
use proteus_lib::dsp::effects::{
    AudioEffect, BasicReverbEffect, ConvolutionReverbEffect, DistortionEffect,
    HighPassFilterEffect, LowPassFilterEffect,
};
use proteus_lib::playback::player;
use ratatui::{backend::CrosstermBackend, Terminal};
use serde::Serialize;
use symphonia::core::errors::Result;

use crate::logging::LogLine;
use crate::{cli, controls, logging, ui};

/// Main CLI execution path: parse args, run benches, or start playback.
pub fn run(args: &ArgMatches, log_buffer: Arc<Mutex<VecDeque<LogLine>>>) -> Result<i32> {
    info!("Starting Proteus CLI");
    // Primary entry for CLI execution; runs benchmarks or playback.
    if let Some((subcommand, sub_args)) = args.subcommand() {
        let code = match subcommand {
            "info" => {
                let file_path = sub_args.get_one::<String>("INPUT").unwrap();
                let print = sub_args.get_flag("print");
                run_info(file_path, print)
            }
            "peaks" => {
                let file_path = sub_args.get_one::<String>("INPUT").unwrap();
                let limited = sub_args.get_flag("limited");
                run_peaks(file_path, limited)
            }
            "create" => match sub_args.subcommand() {
                Some(("effects-json", _)) => run_create_effects_json(),
                _ => {
                    error!("Unknown create subcommand");
                    -1
                }
            },
            _ => {
                error!("Unknown subcommand");
                -1
            }
        };
        return Ok(code);
    }
    if let Some(code) = cli::bench::maybe_run_bench(args)? {
        return Ok(code);
    }

    let file_path = match args.get_one::<String>("INPUT") {
        Some(path) => path.clone(),
        None => {
            error!("Missing input file path");
            return Ok(-1);
        }
    };
    if args.get_flag("scan-durations") {
        let start = std::time::Instant::now();
        let durations = proteus_lib::container::info::get_durations_by_scan(&file_path);
        let elapsed = start.elapsed();
        let mut items = durations.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        for (track_id, seconds) in items {
            println!("track {}: {:.3}s", track_id, seconds);
        }
        println!("scan duration: {:.3}s", elapsed.as_secs_f64());
        return Ok(0);
    }
    if args.get_flag("read-durations") {
        let start = std::time::Instant::now();
        let durations = proteus_lib::container::info::get_durations(&file_path);
        let elapsed = start.elapsed();
        let mut items = durations.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        for (track_id, seconds) in items {
            println!("track {}: {:.3}s", track_id, seconds);
        }
        println!("scan duration: {:.3}s", elapsed.as_secs_f64());
        return Ok(0);
    }
    let gain = args
        .get_one::<String>("GAIN")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    let quiet = args.get_flag("quiet");

    if !(file_path.ends_with(".prot") || file_path.ends_with(".mka")) {
        error!("File is not a .prot or .mka file");
        return Ok(-1);
    }

    let mut player = player::Player::new(&file_path);
    let start_buffer_ms = args
        .get_one::<String>("start-buffer-ms")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    player.set_start_buffer_ms(start_buffer_ms);
    let track_eos_ms = args
        .get_one::<String>("track-eos-ms")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    player.set_track_eos_ms(track_eos_ms);
    let effects_json_path = args.get_one::<String>("effects-json").cloned();
    if let Some(path) = effects_json_path.as_deref() {
        match load_effects_json(path) {
            Ok(effects) => player.set_effects(effects),
            Err(err) => {
                error!("Failed to load effects json: {}", err);
                return Ok(-1);
            }
        }
    }
    if let Some(impulse_response) = args.get_one::<String>("impulse-response") {
        player.set_impulse_response_from_string(impulse_response);
    }

    let reverb_mix_source = args.value_source("reverb-mix");
    let should_apply_reverb_mix =
        effects_json_path.is_none() || matches!(reverb_mix_source, Some(clap::parser::ValueSource::CommandLine));
    if should_apply_reverb_mix {
        let reverb_mix = args
            .get_one::<String>("reverb-mix")
            .unwrap()
            .parse::<f32>()
            .unwrap();
        player.set_reverb_mix(reverb_mix);
    }

    // Start playback once configuration is applied.
    player.play();
    player.set_volume(gain / 100.0);

    let _raw_mode = RawModeGuard::enable().ok();
    let mut terminal = if !quiet {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, EnterAlternateScreen, cursor::Hide);
        let backend = CrosstermBackend::new(stdout);
        Terminal::new(backend).ok()
    } else {
        None
    };
    let _stderr_guard = if terminal.is_some() {
        logging::capture_stderr(log_buffer.clone())
    } else {
        None
    };

    // UI / input loop.
    while !player.is_finished() {
        if let Some(term) = terminal.as_mut() {
            let time = player.get_time();
            let duration = player.get_duration();
            let playing = player.is_playing();
            let effect_names = player.get_effect_names();
            #[cfg(feature = "output-meter")]
            let levels = player.get_levels();
            #[cfg(not(feature = "output-meter"))]
            let levels: Vec<f32> = Vec::new();
            #[cfg(feature = "debug")]
            let dsp_metrics = player.get_dsp_metrics();
            #[cfg(feature = "debug")]
            let (thread_exists, state, audio_heard) = player.debug_playback_state();
            #[cfg(feature = "debug")]
            let buffering_done = player.debug_buffering_done();
            #[cfg(feature = "debug")]
            let (_sink_paused, _sink_empty, sink_len) = player.debug_sink_state();
            let log_lines = logging::snapshot_lines(&log_buffer);
            let status = controls::status_text(controls::StatusArgs {
                time,
                duration,
                playing,
                effects: effect_names,
                #[cfg(feature = "debug")]
                sample_rate: player.info.sample_rate,
                #[cfg(feature = "debug")]
                channels: player.info.channels,
                #[cfg(feature = "debug")]
                dsp_time_ms: dsp_metrics.dsp_time_ms,
                #[cfg(feature = "debug")]
                audio_time_ms: dsp_metrics.audio_time_ms,
                #[cfg(feature = "debug")]
                rt_factor: dsp_metrics.rt_factor,
                #[cfg(feature = "debug")]
                chain_ksps: dsp_metrics.chain_ksps,
                #[cfg(feature = "debug")]
                avg_chain_ksps: dsp_metrics.avg_chain_ksps,
                #[cfg(feature = "debug")]
                min_chain_ksps: dsp_metrics.min_chain_ksps,
                #[cfg(feature = "debug")]
                max_chain_ksps: dsp_metrics.max_chain_ksps,
                #[cfg(feature = "debug")]
                track_key_count: dsp_metrics.track_key_count,
                #[cfg(feature = "debug")]
                finished_track_count: dsp_metrics.finished_track_count,
                #[cfg(feature = "debug")]
                prot_key_count: dsp_metrics.prot_key_count,
                #[cfg(feature = "debug")]
                thread_exists,
                #[cfg(feature = "debug")]
                state_label: format!("{:?}", state),
                #[cfg(feature = "debug")]
                audio_heard,
                #[cfg(feature = "debug")]
                buffering_done,
                #[cfg(feature = "debug")]
                sink_len,
            });
            ui::draw_status(term, &status, &log_lines, &levels);
        }

        if !controls::handle_key_event(&mut player) {
            break;
        }

        sleep(Duration::from_millis(50));
    }

    // Restore the terminal state before exiting.
    if let Some(mut term) = terminal {
        let _ = term.show_cursor();
        let stdout = term.backend_mut();
        let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    }

    Ok(0)
}

#[derive(Serialize)]
struct PeakWindow {
    max: f32,
    min: f32,
}

#[derive(Serialize)]
struct PeaksChannel {
    peaks: Vec<PeakWindow>,
}

#[derive(Serialize)]
struct PeaksOutput {
    channels: Vec<PeaksChannel>,
}

fn run_peaks(file_path: &str, limited: bool) -> i32 {
    let peaks = proteus_lib::peaks::get_peaks(file_path, limited);
    let channels = peaks
        .into_iter()
        .map(|channel| PeaksChannel {
            peaks: channel
                .into_iter()
                .map(|(max, min)| PeakWindow { max, min })
                .collect(),
        })
        .collect();
    let output = PeaksOutput { channels };
    match serde_json::to_string_pretty(&output) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(err) => {
            error!("Failed to serialize peaks: {}", err);
            -1
        }
    }
}

fn run_create_effects_json() -> i32 {
    let effects = default_effects_chain();
    match serde_json::to_string_pretty(&effects) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(err) => {
            error!("Failed to serialize effects: {}", err);
            -1
        }
    }
}

fn default_effects_chain() -> Vec<AudioEffect> {
    vec![
        AudioEffect::ConvolutionReverb(ConvolutionReverbEffect::default()),
        AudioEffect::BasicReverb(BasicReverbEffect::default()),
        AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
        AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
        AudioEffect::Distortion(DistortionEffect::default()),
    ]
}

fn load_effects_json(path: &str) -> std::result::Result<Vec<AudioEffect>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path, err))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse json: {}", err))
}

fn run_info(file_path: &str, print: bool) -> i32 {
    let info = proteus_lib::container::info::Info::new(file_path.to_string());
    if print {
        println!("File: {}", file_path);
        println!("Tracks: {}", info.duration_map.len());
        println!("Channels: {}", info.channels);
        println!("Sample rate: {} Hz", info.sample_rate);
        println!("Bits per sample: {}", info.bits_per_sample);

        let mut track_items: Vec<(u32, f64)> =
            info.duration_map.iter().map(|(k, v)| (*k, *v)).collect();
        track_items.sort_by(|a, b| a.0.cmp(&b.0));
        if track_items.is_empty() {
            println!("No track durations available.");
        } else {
            for (track_id, seconds) in track_items {
                println!("Track {}: {:.3}s", track_id, seconds);
            }
        }

        return 0;
    }

    let _raw_mode = RawModeGuard::enable().ok();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, EnterAlternateScreen, cursor::Hide);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(err) => {
            error!("Failed to create terminal: {}", err);
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
            return -1;
        }
    };

    loop {
        ui::draw_info(&mut terminal, &info, file_path);
        if let Ok(true) = event::poll(Duration::from_millis(200)) {
            if let Ok(event::Event::Key(key)) = event::read() {
                match key.code {
                    event::KeyCode::Char('q')
                    | event::KeyCode::Esc
                    | event::KeyCode::Enter => break,
                    _ => {}
                }
            }
        }
    }

    let _ = terminal.show_cursor();
    let stdout = terminal.backend_mut();
    let _ = crossterm::execute!(stdout, LeaveAlternateScreen, cursor::Show);

    0
}

/// RAII guard for terminal raw mode.
struct RawModeGuard;

impl RawModeGuard {
    /// Enable raw mode and return a guard that restores it on drop.
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    /// Restore terminal state.
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
