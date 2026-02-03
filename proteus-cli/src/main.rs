//! # Prot Play
//!
//! A command-line audio player for the Prot audio format.
use std::{io, thread::sleep, time::Duration};

use clap::{Arg, ArgMatches};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{debug, error};
use proteus_lib::{player, test_data};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use symphonia::core::errors::Result;

/// The main entry point of the application.
fn main() {
    let args = clap::Command::new("Prot Play")
        .version("1.0")
        .author("Adam Howard <adam.thomas.howard@gmail.com>")
        .about("Play Prot audio")
        .arg(
            Arg::new("seek")
                .long("seek")
                .short('s')
                .value_name("TIME")
                .help("Seek to the given time in seconds")
                .conflicts_with_all(&["verify", "decode-only", "verify-only", "probe-only"]),
        )
        .arg(
            Arg::new("GAIN")
                .long("gain")
                .short('g')
                .value_name("GAIN")
                .default_value("70")
                // .min(0)
                // .max(100)
                .help("The playback gain"),
        )
        .arg(
            Arg::new("decode-only")
                .long("decode-only")
                .help("Decode, but do not play the audio")
                .conflicts_with_all(&["probe-only", "verify-only", "verify"]),
        )
        .arg(
            Arg::new("probe-only")
                .long("probe-only")
                .help("Only probe the input for metadata")
                .conflicts_with_all(&["decode-only", "verify-only"]),
        )
        .arg(
            Arg::new("verify-only")
                .long("verify-only")
                .help("Verify the decoded audio is valid, but do not play the audio")
                .conflicts_with_all(&["verify"]),
        )
        .arg(
            Arg::new("verify")
                .long("verify")
                .short('v')
                .help("Verify the decoded audio is valid during playback"),
        )
        .arg(
            Arg::new("no-progress")
                .long("no-progress")
                .help("Do not display playback progress"),
        )
        .arg(
            Arg::new("no-gapless")
                .long("no-gapless")
                .help("Disable gapless decoding and playback"),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(clap::ArgAction::SetTrue)
                .help("Suppress all console output"),
        )
        .arg(Arg::new("debug").short('d').help("Show debug output"))
        .arg(
            Arg::new("INPUT")
                .help("The input file path, or - to use standard input")
                .required(true)
                .index(1),
        )
        .get_matches();

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

/// Formats a given time in seconds into a HH:MM:SS format.
///
/// # Arguments
///
/// * `time` - The time in seconds to format.
fn format_time(time: f64) -> String {
    // Seconds rounded up
    let seconds = (time / 1000.0).ceil() as u32;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
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

    let test_data = test_data::TestData::new();
    let mut player = player::Player::new_from_file_paths(&test_data.wavs);
    if !quiet {
        debug!("Test info: {:?}", player.info);
    }

    // let test_info = info::Info::new_from_file_paths(test_data.wavs);
    // println!("Duration: {}", format_time(info.get_duration(0).unwrap() * 1000.0));

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

    const SEEK_STEP_SECONDS: f64 = 5.0;

    // player.pause();

    while !player.is_finished() {
        if let Some(term) = terminal.as_mut() {
            let time = player.get_time();
            let duration = player.get_duration();
            let playing = player.is_playing();
            let state = if playing { "▶ Playing" } else { "⏸ Paused" };
            let current = format_time(time * 1000.0);
            let total = format_time(duration * 1000.0);
            let percent = if duration > 0.0 {
                (time / duration * 100.0).min(100.0)
            } else {
                0.0
            };
            let status = format!("{}   {} / {}   ({:>5.1}%)", state, current, total, percent);

            let _ = term.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints([
                        Constraint::Length(5),
                        Constraint::Length(3),
                        Constraint::Min(0),
                    ])
                    .split(f.size());

                let controls = Paragraph::new("space=play/pause  s=shuffle  ←/→=seek 5s  q=quit")
                    .style(Style::default().fg(Color::Blue))
                    .block(Block::default().borders(Borders::ALL).title("Controls"));
                f.render_widget(controls, chunks[0]);

                let status_widget = Paragraph::new(status)
                    .style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                    .block(Block::default().borders(Borders::ALL).title("Playback"));
                f.render_widget(status_widget, chunks[1]);
            });
        }

        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => {
                        player.stop();
                        break;
                    }
                    KeyCode::Char(' ') => {
                        if player.is_playing() {
                            player.pause();
                        } else {
                            player.resume();
                        }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        player.shuffle();
                    }
                    KeyCode::Left => {
                        let current = player.get_time();
                        let target = (current - SEEK_STEP_SECONDS).max(0.0);
                        player.seek(target);
                    }
                    KeyCode::Right => {
                        let current = player.get_time();
                        let duration = player.get_duration();
                        let target = (current + SEEK_STEP_SECONDS).min(duration);
                        player.seek(target);
                    }
                    _ => {}
                }
            }
        }

        sleep(Duration::from_millis(50));
    }

    if let Some(mut term) = terminal {
        let _ = term.show_cursor();
        let mut stdout = term.backend_mut();
        let _ = execute!(stdout, LeaveAlternateScreen, cursor::Show);
    }
    Ok(0)
}
