//! Ratatui UI layout and rendering helpers.

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use text_to_ascii_art::to_art;

use crate::controls::StatusSnapshot;
use crate::logging::{LogKind, LogLine};
use proteus_lib::container::info::Info;

/// Render the TUI frame (title, controls, status, logs).
pub fn draw_status(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    status: &StatusSnapshot,
    log_lines: &[LogLine],
    levels: &[f32],
    levels_db: &[f32],
) {
    // Render the controls + status panels.
    let _ = terminal.draw(|f| {
        let status_height = {
            #[cfg(feature = "debug")]
            {
                16
            }
            #[cfg(not(feature = "debug"))]
            {
                4
            }
        };
        let title_text = to_art("Proteus".to_string(), "standard", 0, 1, 0)
            .unwrap_or_else(|_| "Proteus Audio".to_string());
        let title_height = title_text.lines().count().max(1) as u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(title_height),
                Constraint::Length(3),
                Constraint::Length(status_height),
                Constraint::Min(0),
            ])
            .split(f.size());

        let title = Paragraph::new(title_text).style(Style::default().fg(Color::Cyan));

        f.render_widget(title, chunks[0]);

        let controls = Paragraph::new(
            "\nspace=play/pause  s=shuffle  ←/→=seek 5s  r=reverb on/off  -/= mix  q=quit",
        )
        .style(Style::default().fg(Color::Blue));
        f.render_widget(controls, chunks[1]);

        let status_area = chunks[2];
        #[cfg(feature = "output-meter")]
        {
            let meter_mode = pick_meter_mode(status_area.width, status_area.height);
            match meter_mode {
                MeterMode::Hidden => {
                    let status_widget = Paragraph::new(status.text.as_str())
                        .style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )
                        .block(Block::default().borders(Borders::ALL).title("Playback"));
                    f.render_widget(status_widget, status_area);
                }
                MeterMode::Vertical => {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(0), Constraint::Length(10)])
                        .split(status_area);

                    let status_widget = Paragraph::new(status.text.as_str())
                        .style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )
                        .block(Block::default().borders(Borders::ALL).title("Playback"));
                    f.render_widget(status_widget, cols[0]);

                    let meter_text =
                        vertical_meter_text(levels, levels_db, cols[1].height.saturating_sub(2));
                    let meter_widget = Paragraph::new(meter_text)
                        .style(Style::default().fg(Color::Cyan))
                        .block(Block::default().borders(Borders::ALL).title("Levels"));
                    f.render_widget(meter_widget, cols[1]);
                }
                MeterMode::Horizontal => {
                    let rows = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(4)])
                        .split(status_area);

                    let status_widget = Paragraph::new(status.text.as_str())
                        .style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )
                        .block(Block::default().borders(Borders::ALL).title("Playback"));
                    f.render_widget(status_widget, rows[0]);

                    let meter_text =
                        horizontal_meter_text(levels, levels_db, rows[1].width.saturating_sub(2));
                    let meter_widget = Paragraph::new(meter_text)
                        .style(Style::default().fg(Color::Cyan))
                        .block(Block::default().borders(Borders::ALL).title("Levels"));
                    f.render_widget(meter_widget, rows[1]);
                }
            }
        }
        #[cfg(not(feature = "output-meter"))]
        {
            let _ = levels;
            let _ = levels_db;
            let status_widget = Paragraph::new(status.text.as_str())
                .style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
                .block(Block::default().borders(Borders::ALL).title("Playback"));
            f.render_widget(status_widget, status_area);
        }

        let log_height = chunks[3].height.saturating_sub(2) as usize;
        let start = log_lines.len().saturating_sub(log_height);
        let log_text = if log_lines.is_empty() {
            Text::from(Line::styled(
                "No logs yet.",
                Style::default().fg(Color::DarkGray),
            ))
        } else {
            let lines: Vec<Line> = log_lines[start..]
                .iter()
                .map(|line| {
                    let color = match line.kind {
                        LogKind::Error | LogKind::Stderr => Color::Red,
                        LogKind::Warn => Color::Yellow,
                        LogKind::Info => Color::DarkGray,
                        LogKind::Debug => Color::Blue,
                        LogKind::Trace => Color::Magenta,
                    };
                    Line::styled(line.text.as_str(), Style::default().fg(color))
                })
                .collect();
            Text::from(lines)
        };

        let log_widget =
            Paragraph::new(log_text).block(Block::default().borders(Borders::ALL).title("Logs"));
        f.render_widget(log_widget, chunks[3]);
    });
}

#[cfg(feature = "output-meter")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeterMode {
    Hidden,
    Vertical,
    Horizontal,
}

#[cfg(feature = "output-meter")]
fn pick_meter_mode(width: u16, height: u16) -> MeterMode {
    if width >= 70 && height >= 8 {
        return MeterMode::Vertical;
    }
    if width >= 46 && height >= 6 {
        return MeterMode::Horizontal;
    }
    MeterMode::Hidden
}

#[cfg(feature = "output-meter")]
fn vertical_meter_text(levels: &[f32], levels_db: &[f32], height: u16) -> Text<'static> {
    let display = pick_levels(levels);
    let display_db = pick_levels_db(levels_db);
    let mut lines: Vec<Line> = Vec::new();
    let h = height.max(1) as usize;
    let l_fill = (display.left * h as f32).ceil() as usize;
    let r_fill = (display.right * h as f32).ceil() as usize;

    for row in (0..h).rev() {
        let l_on = row < l_fill;
        let r_on = row < r_fill;
        let line = format!(" {} {}", if l_on { "#" } else { " " }, if r_on { "#" } else { " " });
        lines.push(Line::from(line));
    }

    let label = if display.has_right { " L R" } else { "  M" };
    lines.push(Line::from(label));
    let db_label = if display_db.has_right {
        format!("{} {} dB", format_db(display_db.left), format_db(display_db.right))
    } else {
        format!(" {} dB", format_db(display_db.left))
    };
    lines.push(Line::from(db_label));
    Text::from(lines)
}

#[cfg(feature = "output-meter")]
fn horizontal_meter_text(levels: &[f32], levels_db: &[f32], width: u16) -> Text<'static> {
    let display = pick_levels(levels);
    let display_db = pick_levels_db(levels_db);
    let bar_width = width.saturating_sub(6).max(4) as usize;

    let left = render_bar(display.left, bar_width);
    let mut lines = vec![Line::from(format!(
        "L [{}] {} dB",
        left,
        format_db(display_db.left)
    ))];

    if display.has_right {
        let right = render_bar(display.right, bar_width);
        lines.push(Line::from(format!(
            "R [{}] {} dB",
            right,
            format_db(display_db.right)
        )));
    } else {
        let mono = render_bar(display.left, bar_width);
        lines.push(Line::from(format!(
            "M [{}] {} dB",
            mono,
            format_db(display_db.left)
        )));
    }

    Text::from(lines)
}

#[cfg(feature = "output-meter")]
fn render_bar(level: f32, width: usize) -> String {
    let clamped = level.clamp(0.0, 1.0);
    let filled = (clamped * width as f32).round() as usize;
    let mut out = String::with_capacity(width);
    for i in 0..width {
        out.push(if i < filled { '#' } else { ' ' });
    }
    out
}

#[cfg(feature = "output-meter")]
struct LevelDisplay {
    left: f32,
    right: f32,
    has_right: bool,
}

#[cfg(feature = "output-meter")]
struct LevelDbDisplay {
    left: f32,
    right: f32,
    has_right: bool,
}

#[cfg(feature = "output-meter")]
fn pick_levels(levels: &[f32]) -> LevelDisplay {
    if levels.len() >= 2 {
        LevelDisplay {
            left: levels[0].clamp(0.0, 1.0),
            right: levels[1].clamp(0.0, 1.0),
            has_right: true,
        }
    } else if let Some(value) = levels.first().copied() {
        LevelDisplay {
            left: value.clamp(0.0, 1.0),
            right: 0.0,
            has_right: false,
        }
    } else {
        LevelDisplay {
            left: 0.0,
            right: 0.0,
            has_right: false,
        }
    }
}

#[cfg(feature = "output-meter")]
fn pick_levels_db(levels: &[f32]) -> LevelDbDisplay {
    if levels.len() >= 2 {
        LevelDbDisplay {
            left: levels[0],
            right: levels[1],
            has_right: true,
        }
    } else if let Some(value) = levels.first().copied() {
        LevelDbDisplay {
            left: value,
            right: 0.0,
            has_right: false,
        }
    } else {
        LevelDbDisplay {
            left: f32::NEG_INFINITY,
            right: f32::NEG_INFINITY,
            has_right: false,
        }
    }
}

#[cfg(feature = "output-meter")]
fn format_db(value: f32) -> String {
    if value.is_infinite() && value.is_sign_negative() {
        "-inf".to_string()
    } else {
        format!("{value:>5.1}")
    }
}

/// Render the TUI frame for container info.
pub fn draw_info(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    info: &Info,
    file_path: &str,
) {
    let _ = terminal.draw(|f| {
        let title_text = to_art("Proteus".to_string(), "standard", 0, 1, 0)
            .unwrap_or_else(|_| "Proteus Audio".to_string());
        let title_height = title_text.lines().count().max(1) as u16;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(title_height),
                Constraint::Length(8),
                Constraint::Min(6),
                Constraint::Length(2),
            ])
            .split(f.size());

        let title = Paragraph::new(title_text).style(Style::default().fg(Color::Cyan));
        f.render_widget(title, chunks[0]);

        let summary_lines = vec![
            Line::from(format!("File: {}", file_path)),
            Line::from(format!("Tracks: {}", info.duration_map.len())),
            Line::from(format!("Channels: {}", info.channels)),
            Line::from(format!("Sample rate: {} Hz", info.sample_rate)),
            Line::from(format!("Bits per sample: {}", info.bits_per_sample)),
        ];
        let summary = Paragraph::new(Text::from(summary_lines))
            .style(Style::default().fg(Color::Green))
            .block(Block::default().borders(Borders::ALL).title("Summary"));
        f.render_widget(summary, chunks[1]);

        let mut track_items: Vec<(u32, f64)> =
            info.duration_map.iter().map(|(k, v)| (*k, *v)).collect();
        track_items.sort_by(|a, b| a.0.cmp(&b.0));
        let track_lines: Vec<Line> = if track_items.is_empty() {
            vec![Line::from("No track durations available.")]
        } else {
            track_items
                .into_iter()
                .map(|(track_id, seconds)| {
                    Line::from(format!("Track {}: {:.3}s", track_id, seconds))
                })
                .collect()
        };
        let tracks = Paragraph::new(Text::from(track_lines))
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).title("Track Durations"));
        f.render_widget(tracks, chunks[2]);

        let footer = Paragraph::new("Press q, Esc, or Enter to exit")
            .style(Style::default().fg(Color::Blue));
        f.render_widget(footer, chunks[3]);
    });
}
