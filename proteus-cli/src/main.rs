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
use log::error;
use proteus_lib::playback::player;
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
            Arg::new("impulse-response")
                .long("ir")
                .value_name("IMPULSE")
                .help(
                    "Impulse response path or attachment (e.g., file:ir.wav or attachment:ir.wav)",
                ),
        )
        .arg(
            Arg::new("bench-dsp")
                .long("bench-dsp")
                .action(clap::ArgAction::SetTrue)
                .help("Run a synthetic DSP benchmark and exit"),
        )
        .arg(
            Arg::new("bench-sweep")
                .long("bench-sweep")
                .action(clap::ArgAction::SetTrue)
                .help("Run a sweep over multiple FFT sizes and exit"),
        )
        .arg(
            Arg::new("bench-fft-size")
                .long("bench-fft-size")
                .value_name("SIZE")
                .default_value("24576")
                .help("FFT size for DSP benchmark"),
        )
        .arg(
            Arg::new("bench-input-seconds")
                .long("bench-input-seconds")
                .value_name("SECONDS")
                .default_value("1.0")
                .help("Input length in seconds for DSP benchmark"),
        )
        .arg(
            Arg::new("bench-ir-seconds")
                .long("bench-ir-seconds")
                .value_name("SECONDS")
                .default_value("2.0")
                .help("Impulse response length in seconds for DSP benchmark"),
        )
        .arg(
            Arg::new("bench-iterations")
                .long("bench-iterations")
                .value_name("COUNT")
                .default_value("5")
                .help("Number of iterations for DSP benchmark"),
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
                .required_unless_present_any(["bench-dsp", "bench-sweep"])
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
    if args.get_flag("bench-dsp") {
        #[cfg(not(feature = "bench"))]
        {
            eprintln!("Benchmarking requires the `bench` feature.");
            return Ok(1);
        }
        #[cfg(feature = "bench")]
        {
            let fft_size = args
                .get_one::<String>("bench-fft-size")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let input_seconds = args
                .get_one::<String>("bench-input-seconds")
                .unwrap()
                .parse::<f32>()
                .unwrap();
            let ir_seconds = args
                .get_one::<String>("bench-ir-seconds")
                .unwrap()
                .parse::<f32>()
                .unwrap();
            let iterations = args
                .get_one::<String>("bench-iterations")
                .unwrap()
                .parse::<usize>()
                .unwrap();

            let result = proteus_lib::diagnostics::bench::bench_convolver(
                proteus_lib::diagnostics::bench::DspBenchConfig {
                    sample_rate: 44_100,
                    input_seconds,
                    ir_seconds,
                    fft_size,
                    iterations,
                },
            );

            println!(
            "DSP bench (fft={} input={}s ir={}s iters={}): avg {:.2}ms (min {:.2}ms max {:.2}ms), audio {:.2}ms, rt {:.2}x, ir_segments {}",
            fft_size,
            input_seconds,
            ir_seconds,
            iterations,
            result.avg_ms,
            result.min_ms,
            result.max_ms,
            result.audio_time_ms,
            result.rt_factor,
            result.ir_segments
        );

            return Ok(0);
        }
    }

    if args.get_flag("bench-sweep") {
        #[cfg(not(feature = "bench"))]
        {
            eprintln!("Benchmarking requires the `bench` feature.");
            return Ok(1);
        }
        #[cfg(feature = "bench")]
        {
            let fft_sizes = [8192, 12288, 16384, 20480, 24576, 32768];
            let input_seconds = args
                .get_one::<String>("bench-input-seconds")
                .unwrap()
                .parse::<f32>()
                .unwrap();
            let ir_seconds = args
                .get_one::<String>("bench-ir-seconds")
                .unwrap()
                .parse::<f32>()
                .unwrap();
            let iterations = args
                .get_one::<String>("bench-iterations")
                .unwrap()
                .parse::<usize>()
                .unwrap();

            let base = proteus_lib::diagnostics::bench::DspBenchConfig {
                sample_rate: 44_100,
                input_seconds,
                ir_seconds,
                fft_size: fft_sizes[0],
                iterations,
            };

            let results = proteus_lib::diagnostics::bench::bench_convolver_sweep(base, &fft_sizes);
            println!(
                "DSP sweep (input={}s ir={}s iters={})",
                input_seconds, ir_seconds, iterations
            );
            println!("fft_size | avg_ms | min_ms | max_ms | rt_x | ir_segments");
            for (fft_size, result) in results {
                println!(
                    "{:>7} | {:>6.2} | {:>6.2} | {:>6.2} | {:>4.2} | {:>11}",
                    fft_size,
                    result.avg_ms,
                    result.min_ms,
                    result.max_ms,
                    result.rt_factor,
                    result.ir_segments
                );
            }
            return Ok(0);
        }
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
    if let Some(impulse_response) = args.get_one::<String>("impulse-response") {
        player.set_impulse_response_from_string(impulse_response);
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
            let reverb_settings = player.get_reverb_settings();
            let reverb_metrics = player.get_reverb_metrics();
            let reverb_state = if reverb_settings.enabled { "on" } else { "off" };
            let status = format!(
                "{}   {} / {}   ({:>5.1}%)\nReverb: {} | mix: {:.2}\nDSP: {:.2}ms / {:.2}ms ({:.2}x)\nAVG: {:.2}ms / {:.2}ms ({:.2}x)  MIN/MAX: {:.2}/{:.2}x",
                state,
                current,
                total,
                percent,
                reverb_state,
                reverb_settings.dry_wet,
                reverb_metrics.dsp_time_ms,
                reverb_metrics.audio_time_ms,
                reverb_metrics.rt_factor,
                reverb_metrics.avg_dsp_ms,
                reverb_metrics.avg_audio_ms,
                reverb_metrics.avg_rt_factor,
                reverb_metrics.min_rt_factor,
                reverb_metrics.max_rt_factor
            );

            let _ = term.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints([
                        Constraint::Length(6),
                        Constraint::Length(6),
                        Constraint::Min(0),
                    ])
                    .split(f.size());

                let controls = Paragraph::new(
                    "space=play/pause  s=shuffle  ←/→=seek 5s  r=reverb on/off  -/= mix  q=quit",
                )
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
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        let settings = player.get_reverb_settings();
                        player.set_reverb_enabled(!settings.enabled);
                    }
                    KeyCode::Char('-') => {
                        let settings = player.get_reverb_settings();
                        let next = (settings.dry_wet - 0.05).max(0.0);
                        player.set_reverb_mix(next);
                    }
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        let settings = player.get_reverb_settings();
                        let next = (settings.dry_wet + 0.05).min(1.0);
                        player.set_reverb_mix(next);
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
