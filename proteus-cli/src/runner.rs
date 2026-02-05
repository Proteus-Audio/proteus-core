use std::{collections::VecDeque, io, sync::{Arc, Mutex}, thread::sleep, time::Duration};

use clap::ArgMatches;
use crossterm::{
    cursor, execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::error;
use proteus_lib::playback::player;
use ratatui::{backend::CrosstermBackend, Terminal};
use symphonia::core::errors::Result;

use crate::{cli, controls, logging, ui};

pub fn run(args: &ArgMatches, log_buffer: Arc<Mutex<VecDeque<String>>>) -> Result<i32> {
    // Primary entry for CLI execution; runs benchmarks or playback.
    if let Some(code) = cli::bench::maybe_run_bench(args)? {
        return Ok(code);
    }

    let file_path = args.get_one::<String>("INPUT").unwrap().clone();
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
    if let Some(impulse_response) = args.get_one::<String>("impulse-response") {
        player.set_impulse_response_from_string(impulse_response);
    }

    let reverb_mix = args
        .get_one::<String>("reverb-mix")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    player.set_reverb_mix(reverb_mix);

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

    // UI / input loop.
    while !player.is_finished() {
        if let Some(term) = terminal.as_mut() {
            let time = player.get_time();
            let duration = player.get_duration();
            let playing = player.is_playing();
            let reverb_settings = player.get_reverb_settings();
            #[cfg(feature = "debug")]
            let reverb_metrics = player.get_reverb_metrics();
            let log_lines = logging::snapshot(&log_buffer);
            let status = controls::status_text(controls::StatusArgs {
                time,
                duration,
                playing,
                reverb_state: reverb_settings.enabled,
                reverb_mix: reverb_settings.dry_wet,
                #[cfg(feature = "debug")]
                sample_rate: player.info.sample_rate,
                #[cfg(feature = "debug")]
                dsp_time_ms: reverb_metrics.dsp_time_ms,
                #[cfg(feature = "debug")]
                audio_time_ms: reverb_metrics.audio_time_ms,
                #[cfg(feature = "debug")]
                rt_factor: reverb_metrics.rt_factor,
                #[cfg(feature = "debug")]
                avg_dsp_ms: reverb_metrics.avg_dsp_ms,
                #[cfg(feature = "debug")]
                avg_audio_ms: reverb_metrics.avg_audio_ms,
                #[cfg(feature = "debug")]
                avg_rt_factor: reverb_metrics.avg_rt_factor,
                #[cfg(feature = "debug")]
                min_rt_factor: reverb_metrics.min_rt_factor,
                #[cfg(feature = "debug")]
                max_rt_factor: reverb_metrics.max_rt_factor,
                #[cfg(feature = "debug")]
                buffer_fill: reverb_metrics.buffer_fill,
                #[cfg(feature = "debug")]
                avg_buffer_fill: reverb_metrics.avg_buffer_fill,
                #[cfg(feature = "debug")]
                min_buffer_fill: reverb_metrics.min_buffer_fill,
                #[cfg(feature = "debug")]
                max_buffer_fill: reverb_metrics.max_buffer_fill,
                #[cfg(feature = "debug")]
                chain_time_ms: reverb_metrics.chain_time_ms,
                #[cfg(feature = "debug")]
                avg_chain_time_ms: reverb_metrics.avg_chain_time_ms,
                #[cfg(feature = "debug")]
                min_chain_time_ms: reverb_metrics.min_chain_time_ms,
                #[cfg(feature = "debug")]
                max_chain_time_ms: reverb_metrics.max_chain_time_ms,
                #[cfg(feature = "debug")]
                out_interval_ms: reverb_metrics.out_interval_ms,
                #[cfg(feature = "debug")]
                avg_out_interval_ms: reverb_metrics.avg_out_interval_ms,
                #[cfg(feature = "debug")]
                min_out_interval_ms: reverb_metrics.min_out_interval_ms,
                #[cfg(feature = "debug")]
                max_out_interval_ms: reverb_metrics.max_out_interval_ms,
            });
            ui::draw_status(term, &status, &log_lines);
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
