//! # Prot Play
//!
//! A command-line audio player for the Prot audio format.
use std::{io, thread::sleep, time::Duration};

use clap::ArgMatches;
use crossterm::{
    cursor,
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::error;
use proteus_lib::playback::player;
use ratatui::{backend::CrosstermBackend, Terminal};
use symphonia::core::errors::Result;

mod cli;
mod controls;
mod ui;

/// The main entry point of the application.
fn main() {
    let args = cli::args::build_cli().get_matches();

    // For any error, return an exit code -1. Otherwise return the exit code provided.
    let code = match run(&args) {
        Ok(code) => code,
        Err(err) => {
            error!("{}", err.to_string().to_lowercase());
            -1
        }
    };

    std::process::exit(code)
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// The main logic of the application.
///
/// # Arguments
///
/// * `args` - The command-line arguments.
fn run(args: &ArgMatches) -> Result<i32> {
    if let Some(code) = cli::bench::maybe_run_bench(args)? {
        return Ok(code);
    }

    let file_path = args.get_one::<String>("INPUT").unwrap().clone();
    let gain = args
        .get_one::<String>("GAIN")
        .unwrap()
        .parse::<f32>()
        .unwrap()
        .clone();
    let quiet = args.get_flag("quiet");

    // If file is not a .mka file, return an error
    if !(file_path.ends_with(".prot") || file_path.ends_with(".mka")) {
        panic!("File is not a .prot file");
    }

    // let mut player = player::Player::new(&file_path);

    // let info = info::Info::new(file_path);

    let mut player = player::Player::new(&file_path);
    let start_buffer_ms = args
        .get_one::<String>("start-buffer-ms")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    player.set_start_buffer_ms(start_buffer_ms);
    if let Some(impulse_response) = args.get_one::<String>("impulse-response") {
        player.set_impulse_response_from_string(impulse_response);
    }

    // let test_info = info::Info::new_from_file_paths(test_data.wavs);
    // println!("Duration: {}", info.get_duration(0).unwrap());

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

    while !player.is_finished() {
        if let Some(term) = terminal.as_mut() {
            let time = player.get_time();
            let duration = player.get_duration();
            let playing = player.is_playing();
            let reverb_settings = player.get_reverb_settings();
            let reverb_metrics = player.get_reverb_metrics();
            let status = controls::status_text(controls::StatusArgs {
                time,
                duration,
                playing,
                reverb_state: reverb_settings.enabled,
                reverb_mix: reverb_settings.dry_wet,
                dsp_time_ms: reverb_metrics.dsp_time_ms,
                audio_time_ms: reverb_metrics.audio_time_ms,
                rt_factor: reverb_metrics.rt_factor,
                avg_dsp_ms: reverb_metrics.avg_dsp_ms,
                avg_audio_ms: reverb_metrics.avg_audio_ms,
                avg_rt_factor: reverb_metrics.avg_rt_factor,
                min_rt_factor: reverb_metrics.min_rt_factor,
                max_rt_factor: reverb_metrics.max_rt_factor,
            });
            ui::draw_status(term, &status);
        }

        if !controls::handle_key_event(&mut player) {
            break;
        }

        sleep(Duration::from_millis(50));
    }

    if let Some(mut term) = terminal {
        let _ = term.show_cursor();
        let stdout = term.backend_mut();
        let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    }
    Ok(0)
}
