//! Input handling and status summary helpers for the CLI.

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use proteus_lib::playback::player;

/// Render-ready status text for the TUI.
pub struct StatusSnapshot {
    pub text: String,
}

/// Inputs used to build the status text block.
pub struct StatusArgs {
    pub time: f64,
    pub duration: f64,
    pub playing: bool,
    pub effects: Vec<String>,
    #[cfg(feature = "debug")]
    pub sample_rate: u32,
    #[cfg(feature = "debug")]
    pub channels: u32,
    #[cfg(feature = "debug")]
    pub dsp_time_ms: f64,
    #[cfg(feature = "debug")]
    pub audio_time_ms: f64,
    #[cfg(feature = "debug")]
    pub rt_factor: f64,
    #[cfg(feature = "debug")]
    pub track_key_count: usize,
    #[cfg(feature = "debug")]
    pub finished_track_count: usize,
    #[cfg(feature = "debug")]
    pub prot_key_count: usize,
    #[cfg(feature = "debug")]
    pub chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub avg_chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub min_chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub max_chain_ksps: f64,
    #[cfg(feature = "debug")]
    pub thread_exists: bool,
    #[cfg(feature = "debug")]
    pub state_label: String,
    #[cfg(feature = "debug")]
    pub audio_heard: bool,
    #[cfg(feature = "debug")]
    pub buffering_done: bool,
    #[cfg(feature = "debug")]
    pub sink_len: usize,
}

/// Produce the status snapshot string from runtime metrics.
pub fn status_text(args: StatusArgs) -> StatusSnapshot {
    // Create a multi-line status string for the UI panel.
    let state = if args.playing {
        "▶ Playing"
    } else {
        "⏸ Paused"
    };
    let current = format_time(args.time * 1000.0);
    let total = format_time(args.duration * 1000.0);
    let percent = if args.duration > 0.0 {
        (args.time / args.duration * 100.0).min(100.0)
    } else {
        0.0
    };
    let effects_label = if args.effects.is_empty() {
        "none".to_string()
    } else {
        args.effects.join(", ")
    };

    #[cfg(feature = "debug")]
    let text = format!(
        "{}   {} / {}   ({:>5.1}%)\nEffects: {}\nDSP: {:>6.2}ms audio {:>6.2}ms ({:>4.2}x)\nCHAIN: {:>6.2} ksps (avg {:>6.2} min {:>6.2} max {:>6.2})\nTRK: {}/{} (buf {})\nDBG: thread={} state={} heard={} buf_done={} sink_len={}",
        state,
        current,
        total,
        percent,
        effects_label,
        args.dsp_time_ms,
        args.audio_time_ms,
        args.rt_factor,
        args.chain_ksps,
        args.avg_chain_ksps,
        args.min_chain_ksps,
        args.max_chain_ksps,
        args.finished_track_count,
        args.prot_key_count,
        args.track_key_count,
        args.thread_exists,
        args.state_label,
        args.audio_heard,
        args.buffering_done,
        args.sink_len
    );

    #[cfg(not(feature = "debug"))]
    let text = format!(
        "{}   {} / {}   ({:>5.1}%)\nEffects: {}",
        state, current, total, percent, effects_label
    );

    StatusSnapshot { text }
}

/// Handle a single key event and apply it to the player.
/// Returns `false` if the UI should exit.
pub fn handle_key_event(player: &mut player::Player) -> bool {
    // Handle one input event. Returns false when the user requests exit.
    if event::poll(Duration::from_millis(100)).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                return true;
            }
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    player.stop();
                    return false;
                }
                KeyCode::Char('q') => {
                    player.stop();
                    return false;
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
                    let target = (current - 5.0).max(0.0);
                    player.seek(target);
                }
                KeyCode::Right => {
                    let current = player.get_time();
                    let duration = player.get_duration();
                    let target = (current + 5.0).min(duration);
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

    true
}

/// Format a duration in seconds as `MM:SS`.
fn format_time(time: f64) -> String {
    // Format milliseconds into HH:MM:SS.
    let seconds = (time / 1000.0).ceil() as u32;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
