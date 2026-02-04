use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};

use crate::player;

pub struct StatusSnapshot {
    pub text: String,
}

pub struct StatusArgs {
    pub time: f64,
    pub duration: f64,
    pub playing: bool,
    pub reverb_state: bool,
    pub reverb_mix: f32,
    pub dsp_time_ms: f64,
    pub audio_time_ms: f64,
    pub rt_factor: f64,
    pub avg_dsp_ms: f64,
    pub avg_audio_ms: f64,
    pub avg_rt_factor: f64,
    pub min_rt_factor: f64,
    pub max_rt_factor: f64,
}

pub fn status_text(args: StatusArgs) -> StatusSnapshot {
    let state = if args.playing { "▶ Playing" } else { "⏸ Paused" };
    let current = format_time(args.time * 1000.0);
    let total = format_time(args.duration * 1000.0);
    let percent = if args.duration > 0.0 {
        (args.time / args.duration * 100.0).min(100.0)
    } else {
        0.0
    };
    let reverb_state = if args.reverb_state { "on" } else { "off" };
    let text = format!(
        "{}   {} / {}   ({:>5.1}%)\nReverb: {} | mix: {:.2}\nDSP: {:.2}ms / {:.2}ms ({:.2}x)\nAVG: {:.2}ms / {:.2}ms ({:.2}x)  MIN/MAX: {:.2}/{:.2}x",
        state,
        current,
        total,
        percent,
        reverb_state,
        args.reverb_mix,
        args.dsp_time_ms,
        args.audio_time_ms,
        args.rt_factor,
        args.avg_dsp_ms,
        args.avg_audio_ms,
        args.avg_rt_factor,
        args.min_rt_factor,
        args.max_rt_factor
    );

    StatusSnapshot { text }
}

pub fn handle_key_event(player: &mut player::Player) -> bool {
    if event::poll(Duration::from_millis(100)).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                return true;
            }
            match key.code {
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

fn format_time(time: f64) -> String {
    let seconds = (time / 1000.0).ceil() as u32;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
