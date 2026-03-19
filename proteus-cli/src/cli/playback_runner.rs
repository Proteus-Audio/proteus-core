//! Interactive playback command handler.

use std::{
    collections::VecDeque,
    io,
    path::Path,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use clap::ArgMatches;
use crossterm::{
    cursor, execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::error;
use proteus_lib::{
    container::prot::PathsTrack,
    playback::player::{self, EndOfStreamAction, PlayerInitOptions},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use symphonia::core::errors::Result;

use super::{controls, ui};
use crate::logging::LogLine;
use crate::{logging, project_files};

struct SessionConfig {
    gain: f32,
    quiet: bool,
}

/// Handle default playback path.
pub(crate) fn run_playback(
    args: &ArgMatches,
    log_buffer: Arc<Mutex<VecDeque<LogLine>>>,
) -> Result<i32> {
    let file_path = match args.get_one::<String>("INPUT") {
        Some(path) => path.clone(),
        None => {
            error!("Missing input file path");
            return Ok(-1);
        }
    };
    if let Some(code) = maybe_print_durations(args, &file_path) {
        return Ok(code);
    }

    let session = SessionConfig {
        gain: args
            .get_one::<String>("GAIN")
            .unwrap()
            .parse::<f32>()
            .unwrap(),
        quiet: args.get_flag("quiet"),
    };

    let cli_player_options = PlayerInitOptions {
        end_of_stream_action: EndOfStreamAction::Pause,
    };
    let mut player = build_player_from_args(args, &file_path, cli_player_options)?;

    configure_player(args, &mut player);
    if let Some(path) = args.get_one::<String>("effects-json") {
        match project_files::load_effects_json(path) {
            Ok(effects) => player.set_effects(effects),
            Err(err) => {
                error!("Failed to load effects json: {}", err);
                return Ok(-1);
            }
        }
    }

    player.play();
    player.set_volume(session.gain / 100.0);
    Ok(run_playback_session(player, session, log_buffer))
}

fn maybe_print_durations(args: &ArgMatches, file_path: &str) -> Option<i32> {
    if args.get_flag("scan-durations") {
        let start = std::time::Instant::now();
        let durations = proteus_lib::container::info::get_durations_by_scan(file_path);
        let elapsed = start.elapsed();
        let mut items = durations.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        for (track_id, seconds) in items {
            println!("track {}: {:.3}s", track_id, seconds);
        }
        println!("scan duration: {:.3}s", elapsed.as_secs_f64());
        return Some(0);
    }
    if args.get_flag("read-durations") {
        let start = std::time::Instant::now();
        let durations = proteus_lib::container::info::get_durations(file_path);
        let elapsed = start.elapsed();
        let mut items = durations.into_iter().collect::<Vec<_>>();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        for (track_id, seconds) in items {
            println!("track {}: {:.3}s", track_id, seconds);
        }
        println!("scan duration: {:.3}s", elapsed.as_secs_f64());
        return Some(0);
    }
    None
}

fn build_player_from_args(
    args: &ArgMatches,
    file_path: &str,
    cli_player_options: PlayerInitOptions,
) -> Result<player::Player> {
    let input_path = Path::new(&file_path);
    let is_container = file_path.ends_with(".prot") || file_path.ends_with(".mka");
    let is_directory = input_path.is_dir();
    let player = if is_container {
        player::Player::new_with_options(file_path, cli_player_options)
    } else if is_directory {
        let config = project_files::load_directory_playback_config(input_path).map_err(|err| {
            error!("{}", err);
            symphonia::core::errors::Error::IoError(std::io::Error::other(err))
        })?;
        let mut player =
            player::Player::new_from_file_paths_with_options(config.tracks, cli_player_options);
        if args.get_one::<String>("effects-json").is_none() {
            if let Some(path) = config.effects_json_path {
                match project_files::load_effects_json(path.to_string_lossy().as_ref()) {
                    Ok(effects) => player.set_effects(effects),
                    Err(err) => {
                        return Err(symphonia::core::errors::Error::IoError(
                            std::io::Error::other(format!(
                                "Failed to load effects json from directory: {}",
                                err
                            )),
                        ));
                    }
                }
            }
        }
        player
    } else {
        let track = PathsTrack::new_from_file_paths(vec![file_path.to_string()]);
        player::Player::new_from_file_paths_with_options(vec![track], cli_player_options)
    };
    Ok(player)
}

fn run_playback_session(
    mut player: player::Player,
    config: SessionConfig,
    log_buffer: Arc<Mutex<VecDeque<LogLine>>>,
) -> i32 {
    let _raw_mode = RawModeGuard::enable().ok();
    let mut terminal = if !config.quiet {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, EnterAlternateScreen, cursor::Hide);
        let backend = CrosstermBackend::new(stdout);
        Terminal::new(backend).ok()
    } else {
        None
    };
    logging::set_echo_stderr(!config.quiet && terminal.is_none());
    let _stderr_guard = if terminal.is_some() {
        logging::capture_stderr(log_buffer.clone())
    } else {
        None
    };

    loop {
        if let Some(term) = terminal.as_mut() {
            draw_status_frame(term, &mut player, &log_buffer);
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
    0
}

fn draw_status_frame(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    player: &mut player::Player,
    log_buffer: &Arc<Mutex<VecDeque<LogLine>>>,
) {
    let time = player.get_time();
    let duration = player.get_duration();
    let playing = player.is_playing();
    let effect_names = player.get_effect_names();
    #[cfg(feature = "output-meter")]
    let levels = player.get_levels();
    #[cfg(not(feature = "output-meter"))]
    let levels: Vec<f32> = Vec::new();
    #[cfg(feature = "output-meter")]
    let levels_db = player.get_levels_db();
    #[cfg(not(feature = "output-meter"))]
    let levels_db: Vec<f32> = Vec::new();
    #[cfg(feature = "debug")]
    let dsp_metrics = player.get_dsp_metrics();
    #[cfg(feature = "debug")]
    let (thread_exists, state, audio_heard) = player.debug_playback_state();
    #[cfg(feature = "debug")]
    let buffering_done = player.debug_buffering_done();
    #[cfg(feature = "debug")]
    let (_sink_paused, _sink_empty, sink_len) = player.debug_sink_state();
    let log_lines = logging::snapshot_lines(log_buffer);
    let status = controls::status_text(controls::StatusArgs {
        time,
        duration,
        playing,
        effects: effect_names,
        #[cfg(feature = "debug")]
        sample_rate: player.audio_info().sample_rate,
        #[cfg(feature = "debug")]
        overrun: dsp_metrics.overrun,
        #[cfg(feature = "debug")]
        overrun_ms: dsp_metrics.overrun_ms,
        #[cfg(feature = "debug")]
        avg_overrun_ms: dsp_metrics.avg_overrun_ms,
        #[cfg(feature = "debug")]
        max_overrun_ms: dsp_metrics.max_overrun_ms,
        #[cfg(feature = "debug")]
        chain_ksps: dsp_metrics.chain_ksps,
        #[cfg(feature = "debug")]
        avg_chain_ksps: dsp_metrics.avg_chain_ksps,
        #[cfg(feature = "debug")]
        min_chain_ksps: dsp_metrics.min_chain_ksps,
        #[cfg(feature = "debug")]
        max_chain_ksps: dsp_metrics.max_chain_ksps,
        #[cfg(feature = "debug")]
        underrun_count: dsp_metrics.underrun_count,
        #[cfg(feature = "debug")]
        underrun_active: dsp_metrics.underrun_active,
        #[cfg(feature = "debug")]
        pop_count: dsp_metrics.pop_count,
        #[cfg(feature = "debug")]
        clip_count: dsp_metrics.clip_count,
        #[cfg(feature = "debug")]
        nan_count: dsp_metrics.nan_count,
        #[cfg(feature = "debug")]
        late_append_count: dsp_metrics.late_append_count,
        #[cfg(feature = "debug")]
        late_append_active: dsp_metrics.late_append_active,
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
    ui::draw_status(terminal, &status, &log_lines, &levels, &levels_db);
}

fn arg_f32(args: &ArgMatches, key: &str) -> f32 {
    args.get_one::<String>(key).unwrap().parse::<f32>().unwrap()
}

fn arg_usize(args: &ArgMatches, key: &str) -> usize {
    args.get_one::<String>(key)
        .unwrap()
        .parse::<usize>()
        .unwrap()
}

fn configure_player(args: &ArgMatches, player: &mut player::Player) {
    player.set_start_buffer_ms(arg_f32(args, "start-buffer-ms"));
    player.set_start_sink_chunks(arg_usize(args, "start-sink-chunks"));
    player.set_max_sink_chunks(arg_usize(args, "max-sink-chunks"));
    player.set_startup_silence_ms(arg_f32(args, "startup-silence-ms"));
    player.set_startup_fade_ms(arg_f32(args, "startup-fade-ms"));
    player.set_append_jitter_log_ms(arg_f32(args, "append-jitter-log-ms"));

    player.set_effect_boundary_log(args.get_flag("effect-boundary-log"));
    player.set_track_eos_ms(arg_f32(args, "track-eos-ms"));
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

#[cfg(test)]
mod tests {
    #[test]
    fn maybe_print_durations_returns_none_without_duration_flags() {
        let args = clap::Command::new("prot")
            .arg(
                clap::Arg::new("scan-durations")
                    .long("scan-durations")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                clap::Arg::new("read-durations")
                    .long("read-durations")
                    .action(clap::ArgAction::SetTrue),
            )
            .get_matches_from(["prot"]);
        assert_eq!(super::maybe_print_durations(&args, "ignored"), None);
    }

    #[test]
    fn run_playback_without_input_returns_error_code() {
        let args = clap::Command::new("prot")
            .arg(clap::Arg::new("INPUT").required(false))
            .get_matches_from(["prot"]);
        let logs = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<
            crate::logging::LogLine,
        >::new()));

        let code = super::run_playback(&args, logs).expect("runner should return result");
        assert_eq!(code, -1);
    }
}
