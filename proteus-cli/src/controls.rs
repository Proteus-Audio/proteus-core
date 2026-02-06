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
    pub reverb_state: bool,
    pub reverb_mix: f32,
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
    pub avg_dsp_ms: f64,
    #[cfg(feature = "debug")]
    pub avg_audio_ms: f64,
    #[cfg(feature = "debug")]
    pub avg_rt_factor: f64,
    #[cfg(feature = "debug")]
    pub min_rt_factor: f64,
    #[cfg(feature = "debug")]
    pub max_rt_factor: f64,
    #[cfg(feature = "debug")]
    pub buffer_fill: f64,
    #[cfg(feature = "debug")]
    pub avg_buffer_fill: f64,
    #[cfg(feature = "debug")]
    pub min_buffer_fill: f64,
    #[cfg(feature = "debug")]
    pub max_buffer_fill: f64,
    #[cfg(feature = "debug")]
    pub chain_time_ms: f64,
    #[cfg(feature = "debug")]
    pub avg_chain_time_ms: f64,
    #[cfg(feature = "debug")]
    pub min_chain_time_ms: f64,
    #[cfg(feature = "debug")]
    pub max_chain_time_ms: f64,
    #[cfg(feature = "debug")]
    pub out_interval_ms: f64,
    #[cfg(feature = "debug")]
    pub avg_out_interval_ms: f64,
    #[cfg(feature = "debug")]
    pub min_out_interval_ms: f64,
    #[cfg(feature = "debug")]
    pub max_out_interval_ms: f64,
    #[cfg(feature = "debug")]
    pub wake_total: u64,
    #[cfg(feature = "debug")]
    pub wake_idle: u64,
    #[cfg(feature = "debug")]
    pub dry_rms: f64,
    #[cfg(feature = "debug")]
    pub wet_rms: f64,
    #[cfg(feature = "debug")]
    pub mix_rms: f64,
    #[cfg(feature = "debug")]
    pub dry_peak: f64,
    #[cfg(feature = "debug")]
    pub wet_peak: f64,
    #[cfg(feature = "debug")]
    pub mix_peak: f64,
    #[cfg(feature = "debug")]
    pub wet_to_dry_db: f64,
    #[cfg(feature = "debug")]
    pub reverb_in_len: usize,
    #[cfg(feature = "debug")]
    pub reverb_out_len: usize,
    #[cfg(feature = "debug")]
    pub reverb_reset_gen: u64,
    #[cfg(feature = "debug")]
    pub reverb_block_samples: usize,
    #[cfg(feature = "debug")]
    pub reverb_underflow_events: u64,
    #[cfg(feature = "debug")]
    pub reverb_underflow_samples: u64,
    #[cfg(feature = "debug")]
    pub reverb_pad_events: u64,
    #[cfg(feature = "debug")]
    pub reverb_pad_samples: u64,
    #[cfg(feature = "debug")]
    pub reverb_partial_drain_events: u64,
    #[cfg(feature = "debug")]
    pub append_gap_ms: f64,
    #[cfg(feature = "debug")]
    pub avg_append_gap_ms: f64,
    #[cfg(feature = "debug")]
    pub min_append_gap_ms: f64,
    #[cfg(feature = "debug")]
    pub max_append_gap_ms: f64,
    #[cfg(feature = "debug")]
    pub track_key_count: usize,
    #[cfg(feature = "debug")]
    pub finished_track_count: usize,
    #[cfg(feature = "debug")]
    pub prot_key_count: usize,
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
    let reverb_state = if args.reverb_state { "on" } else { "off" };

    #[cfg(feature = "debug")]
    let ksps = |audio_ms: f64, time_ms: f64, sample_rate: u32, channels: u32| -> f64 {
        if time_ms <= 0.0 || audio_ms <= 0.0 {
            return 0.0;
        }
        let ch = channels.max(1) as f64;
        (audio_ms * sample_rate as f64) / time_ms / 1000.0 / ch
    };
    #[cfg(feature = "debug")]
    let ksps_from_rt = |rt_factor: f64, sample_rate: u32, channels: u32| -> f64 {
        if rt_factor <= 0.0 {
            return 0.0;
        }
        let ch = channels.max(1) as f64;
        (rt_factor * sample_rate as f64) / 1000.0 / ch
    };

    #[cfg(feature = "debug")]
    let text = format!(
        "{}   {} / {}   ({:>5.1}%)\nReverb: {} | mix: {:.2}\nDSP: {:>5.1} ksps / {:>5.1} ksps\nAVG: {:>5.1} ksps / {:>5.1} ksps  MIN/MAX: {:>5.1}/{:>5.1} ksps\nCHAIN: {:>5.1} ksps (avg {:>5.1} min {:>5.1} max {:>5.1})\nOUT: {:>5.1} ksps (avg {:>5.1} min {:>5.1} max {:>5.1})\nBUF: {:.2} (avg {:.2} min {:.2} max {:.2})\nWAKE: {} idle / {} total ({:>5.1}%)\nLVL: dry {:.3}/{:.3} wet {:.3}/{:.3} mix {:.3}/{:.3}  W/D {:>5.1}dB\nRB: in {} out {} gen {}\nRV: block {} underflow {}/{} pad {}/{} partial {}\nAPP: gap {:>6.2}ms avg {:>6.2} min {:>6.2} max {:>6.2}\nTRK: {}/{} (buf {})\nDBG: thread={} state={} heard={} buf_done={} sink_len={}",
        state,
        current,
        total,
        percent,
        reverb_state,
        args.reverb_mix,
        ksps(
            args.audio_time_ms,
            args.dsp_time_ms,
            args.sample_rate,
            args.channels,
        ),
        (args.sample_rate as f64 / 1000.0),
        ksps(
            args.avg_audio_ms,
            args.avg_dsp_ms,
            args.sample_rate,
            args.channels,
        ),
        (args.sample_rate as f64 / 1000.0),
        ksps_from_rt(args.min_rt_factor, args.sample_rate, args.channels),
        ksps_from_rt(args.max_rt_factor, args.sample_rate, args.channels),
        ksps(
            args.audio_time_ms,
            args.chain_time_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.avg_audio_ms,
            args.avg_chain_time_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.audio_time_ms,
            args.min_chain_time_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.audio_time_ms,
            args.max_chain_time_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.audio_time_ms,
            args.out_interval_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.avg_audio_ms,
            args.avg_out_interval_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.audio_time_ms,
            args.min_out_interval_ms,
            args.sample_rate,
            args.channels,
        ),
        ksps(
            args.audio_time_ms,
            args.max_out_interval_ms,
            args.sample_rate,
            args.channels,
        ),
        args.buffer_fill,
        args.avg_buffer_fill,
        args.min_buffer_fill,
        args.max_buffer_fill,
        args.wake_idle,
        args.wake_total,
        if args.wake_total > 0 {
            (args.wake_idle as f64 / args.wake_total as f64) * 100.0
        } else {
            0.0
        },
        args.dry_rms,
        args.dry_peak,
        args.wet_rms,
        args.wet_peak,
        args.mix_rms,
        args.mix_peak,
        args.wet_to_dry_db,
        args.reverb_in_len,
        args.reverb_out_len,
        args.reverb_reset_gen,
        args.reverb_block_samples,
        args.reverb_underflow_events,
        args.reverb_underflow_samples,
        args.reverb_pad_events,
        args.reverb_pad_samples,
        args.reverb_partial_drain_events,
        args.append_gap_ms,
        args.avg_append_gap_ms,
        args.min_append_gap_ms,
        args.max_append_gap_ms,
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
        "{}   {} / {}   ({:>5.1}%)\nReverb: {} | mix: {:.2}",
        state, current, total, percent, reverb_state, args.reverb_mix
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
