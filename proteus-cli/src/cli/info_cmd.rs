//! Info subcommand handlers.

use std::{io, thread::sleep, time::Duration};

use crossterm::{
    cursor, event, execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::error;
use ratatui::{backend::CrosstermBackend, Terminal};

use super::ui;

/// Handle `info` command.
pub(crate) fn run_info(file_path: &str, print: bool) -> i32 {
    let info = proteus_lib::container::info::Info::new(file_path.to_string());
    let audible_duration = adjusted_duration(file_path);
    if print {
        println!("File: {}", file_path);
        println!("Tracks: {}", info.duration_map.len());
        println!("Channels: {}", info.channels);
        println!("Sample rate: {} Hz", info.sample_rate);
        println!("Bits per sample: {}", info.bits_per_sample);

        let mut track_items: Vec<(u32, f64)> =
            info.duration_map.iter().map(|(k, v)| (*k, *v)).collect();
        track_items.sort_by_key(|a| a.0);
        if track_items.is_empty() {
            println!("No track durations available.");
        } else {
            for (track_id, seconds) in track_items {
                println!("Track {} media duration: {:.3}s", track_id, seconds);
            }

            if let Some(seconds) = audible_duration {
                println!("Audible duration (effects-adjusted): {:.3}s", seconds);
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
        ui::draw_info(&mut terminal, &info, file_path, audible_duration);
        if let Ok(true) = event::poll(Duration::from_millis(200)) {
            if let Ok(event::Event::Key(key)) = event::read() {
                match key.code {
                    event::KeyCode::Char('q') | event::KeyCode::Esc | event::KeyCode::Enter => {
                        break;
                    }
                    _ => {}
                }
            }
        }
        sleep(Duration::from_millis(10));
    }

    let _ = terminal.show_cursor();
    let stdout = terminal.backend_mut();
    let _ = crossterm::execute!(stdout, LeaveAlternateScreen, cursor::Show);

    0
}

fn adjusted_duration(file_path: &str) -> Option<f64> {
    if !(file_path.ends_with(".prot") || file_path.ends_with(".mka")) {
        return None;
    }
    proteus_lib::container::prot::Prot::try_new(file_path)
        .ok()
        .map(|mut prot| prot.audible_duration())
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
    use super::run_info;

    #[test]
    fn info_print_mode_returns_success() {
        let code = run_info("/definitely/missing/audio.file", true);
        assert_eq!(code, 0);
    }
}
