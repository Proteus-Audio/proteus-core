//! Ratatui UI layout and rendering helpers.

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

use super::controls::StatusSnapshot;
#[cfg(feature = "effect-meter-cli")]
use super::spectral_graph;
use crate::logging::{LogKind, LogLine};
use proteus_lib::container::info::Info;
use proteus_lib::dsp::meter::{EffectBandSnapshot, EffectLevelSnapshot};

fn title_banner() -> &'static str {
    "Proteus Audio"
}

fn playback_widget(text: &str) -> Paragraph<'_> {
    Paragraph::new(text)
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL).title("Playback"))
}

fn levels_widget(text: Text<'static>) -> Paragraph<'static> {
    Paragraph::new(text)
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL).title("Levels"))
}

#[cfg(feature = "effect-meter-cli")]
fn effect_levels_widget(text: Text<'static>) -> Paragraph<'static> {
    Paragraph::new(text)
        .style(Style::default().fg(Color::Yellow))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Effect Meters"),
        )
}

fn log_text(log_lines: &[LogLine], log_height: usize) -> Text<'_> {
    let start = log_lines.len().saturating_sub(log_height);
    if log_lines.is_empty() {
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
    }
}

/// Render the TUI frame (title, controls, status, logs).
pub fn draw_status(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    status: &StatusSnapshot,
    log_lines: &[LogLine],
    levels: &[f32],
    levels_db: &[f32],
    effect_names: &[String],
    effect_levels: Option<&[EffectLevelSnapshot]>,
    effect_spectral: Option<&[Option<EffectBandSnapshot>]>,
) {
    #[cfg(not(feature = "effect-meter-cli"))]
    {
        let _ = effect_names;
        let _ = effect_levels;
        let _ = effect_spectral;
    }

    // Render the controls + status panels.
    let _ = terminal.draw(|f| {
        let base_status_height = {
            #[cfg(feature = "debug")]
            {
                16
            }
            #[cfg(not(feature = "debug"))]
            {
                4
            }
        };
        #[cfg(feature = "effect-meter-cli")]
        let effect_meter_height =
            effect_meter_panel_height(effect_names, effect_levels, effect_spectral, f.size().width);
        #[cfg(not(feature = "effect-meter-cli"))]
        let effect_meter_height = 0;
        let status_height = base_status_height + effect_meter_height;
        let title_text = format!("{}\nv{}", title_banner(), env!("CARGO_PKG_VERSION"));
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
        #[cfg(feature = "effect-meter-cli")]
        let (playback_area, effect_meter_area) = if effect_meter_height > 0 {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(base_status_height),
                    Constraint::Min(effect_meter_height),
                ])
                .split(status_area);
            (parts[0], Some(parts[1]))
        } else {
            (status_area, None)
        };
        #[cfg(not(feature = "effect-meter-cli"))]
        let playback_area = status_area;
        #[cfg(feature = "output-meter")]
        {
            let meter_mode = pick_meter_mode(playback_area.width, playback_area.height);
            match meter_mode {
                MeterMode::Hidden => {
                    f.render_widget(playback_widget(status.text.as_str()), playback_area);
                }
                MeterMode::Vertical => {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(0), Constraint::Length(10)])
                        .split(playback_area);

                    f.render_widget(playback_widget(status.text.as_str()), cols[0]);

                    let meter_text =
                        vertical_meter_text(levels, levels_db, cols[1].height.saturating_sub(2));
                    f.render_widget(levels_widget(meter_text), cols[1]);
                }
                MeterMode::Horizontal => {
                    let rows = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(4)])
                        .split(playback_area);

                    f.render_widget(playback_widget(status.text.as_str()), rows[0]);

                    let meter_text =
                        horizontal_meter_text(levels, levels_db, rows[1].width.saturating_sub(2));
                    f.render_widget(levels_widget(meter_text), rows[1]);
                }
            }
        }
        #[cfg(not(feature = "output-meter"))]
        {
            let _ = levels;
            let _ = levels_db;
            let _ = effect_names;
            let _ = effect_levels;
            let _ = effect_spectral;
            f.render_widget(playback_widget(status.text.as_str()), playback_area);
        }

        #[cfg(feature = "effect-meter-cli")]
        if let Some(effect_meter_area) = effect_meter_area {
            let meter_text = effect_meter_text(
                effect_names,
                effect_levels,
                effect_spectral,
                effect_meter_area.width,
            );
            f.render_widget(effect_levels_widget(meter_text), effect_meter_area);
        }

        let log_height = chunks[3].height.saturating_sub(2) as usize;
        let log_text = log_text(log_lines, log_height);

        let log_widget =
            Paragraph::new(log_text).block(Block::default().borders(Borders::ALL).title("Logs"));
        f.render_widget(log_widget, chunks[3]);
    });
}

#[cfg(feature = "effect-meter-cli")]
fn effect_meter_panel_height(
    effect_names: &[String],
    effect_levels: Option<&[EffectLevelSnapshot]>,
    effect_spectral: Option<&[Option<EffectBandSnapshot>]>,
    width: u16,
) -> u16 {
    if !spectral_graph::live_effect_meter_visible(
        effect_names,
        effect_levels.map_or(0, <[EffectLevelSnapshot]>::len),
        width,
    ) {
        return 0;
    }
    let visible_rows = effect_rows_len(effect_names, effect_levels, effect_spectral, width);
    visible_rows as u16 + 2
}

#[cfg(feature = "effect-meter-cli")]
fn effect_meter_text(
    effect_names: &[String],
    effect_levels: Option<&[EffectLevelSnapshot]>,
    effect_spectral: Option<&[Option<EffectBandSnapshot>]>,
    width: u16,
) -> Text<'static> {
    let Some(effect_levels) = effect_levels else {
        if effect_names.is_empty() {
            return Text::from(Line::from("No effects configured."));
        }
        return Text::from(Line::from("Effect metering is warming up..."));
    };

    if effect_levels.is_empty() && effect_names.is_empty() {
        return Text::from(Line::from("No effects configured."));
    }

    let line_width = width.saturating_sub(2) as usize;
    let bar_width = if line_width >= 88 {
        10
    } else if line_width >= 72 {
        8
    } else {
        6
    };
    let rows = effect_names
        .len()
        .max(effect_levels.len())
        .max(effect_spectral.map_or(0, <[Option<EffectBandSnapshot>]>::len))
        .min(spectral_graph::EFFECT_METER_MAX_VISIBLE_EFFECTS);
    let show_spectral = spectral_graph::live_spectral_graph_visible(effect_names, width);
    let graph_width = spectral_graph::live_graph_width(width);
    let mut lines = Vec::with_capacity(rows + 1);
    for index in 0..rows {
        let name = effect_names
            .get(index)
            .map(String::as_str)
            .unwrap_or("Effect");
        let snapshot = effect_levels
            .get(index)
            .cloned()
            .unwrap_or_else(EffectLevelSnapshot::default);
        let input_peak = max_level(&snapshot.input.peak);
        let output_peak = max_level(&snapshot.output.peak);
        lines.push(Line::from(format!(
            "[{index}] {:<14} in [{}] {:>5}  out [{}] {:>5}  d {}",
            truncate_ascii(name, 14),
            render_bar(input_peak, bar_width),
            format_db(linear_to_db(input_peak)),
            render_bar(output_peak, bar_width),
            format_db(linear_to_db(output_peak)),
            format_delta_db(linear_to_db(input_peak), linear_to_db(output_peak)),
        )));
        if show_spectral {
            let spectral_snapshot = effect_spectral
                .and_then(|snapshots| snapshots.get(index))
                .and_then(Option::as_ref);
            if spectral_snapshot.is_some() || spectral_graph::supports_spectral_graph(name) {
                let graph = spectral_snapshot
                    .map(|snapshot| {
                        spectral_graph::render_delta_graph(
                            snapshot,
                            graph_width.unwrap_or(line_width),
                        )
                    })
                    .unwrap_or_else(|| {
                        spectral_graph::placeholder_graph(
                            graph_width.unwrap_or(line_width),
                            "warming up",
                        )
                    });
                lines.push(Line::from(format!("    spec d: {graph}")));
            }
        }
    }
    let hidden = effect_names
        .len()
        .max(effect_levels.len())
        .max(effect_spectral.map_or(0, <[Option<EffectBandSnapshot>]>::len))
        .saturating_sub(rows);
    if hidden > 0 {
        lines.push(Line::from(format!("... {} more effect(s)", hidden)));
    }
    Text::from(lines)
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
    let total_height = height.max(1) as usize;
    let meter_height = total_height.saturating_sub(2).max(1);
    let l_fill = (display.left * meter_height as f32).ceil() as usize;
    let r_fill = (display.right * meter_height as f32).ceil() as usize;

    for row in (0..meter_height).rev() {
        let l_on = row < l_fill;
        let r_on = row < r_fill;
        let line = format!(
            " {} {}",
            if l_on { "#" } else { " " },
            if r_on { "#" } else { " " }
        );
        lines.push(Line::from(line));
    }

    let label = if display.has_right { " L R" } else { "  M" };
    lines.push(Line::from(label));
    let db_label = if display_db.has_right {
        format!(
            "{} {} dB",
            format_db(display_db.left),
            format_db(display_db.right)
        )
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
    let line_width = width.max(1) as usize;
    let left_db = format_db(display_db.left);
    let right_db = format_db(display_db.right);
    let overhead = 8; // "L [" + "] " + " dB"
    let left_bar_width = line_width.saturating_sub(overhead + left_db.len()).max(1);
    let right_bar_width = line_width.saturating_sub(overhead + right_db.len()).max(1);

    let left = render_bar(display.left, left_bar_width);
    let mut lines = vec![Line::from(format!("L [{}] {} dB", left, left_db))];

    if display.has_right {
        let right = render_bar(display.right, right_bar_width);
        lines.push(Line::from(format!("R [{}] {} dB", right, right_db)));
    } else {
        let mono = render_bar(display.left, left_bar_width);
        lines.push(Line::from(format!("M [{}] {} dB", mono, left_db)));
    }

    Text::from(lines)
}

#[cfg(any(feature = "output-meter", feature = "effect-meter-cli"))]
fn render_bar(level: f32, width: usize) -> String {
    let clamped = level.clamp(0.0, 1.0);
    let filled = (clamped * width as f32).round() as usize;
    let mut out = String::with_capacity(width);
    for i in 0..width {
        out.push(if i < filled { '#' } else { ' ' });
    }
    out
}

#[cfg(feature = "effect-meter-cli")]
fn max_level(levels: &[f32]) -> f32 {
    levels
        .iter()
        .copied()
        .max_by(|left, right| left.total_cmp(right))
        .unwrap_or(0.0)
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

#[cfg(any(feature = "output-meter", feature = "effect-meter-cli"))]
fn format_db(value: f32) -> String {
    if value.is_infinite() && value.is_sign_negative() {
        "-inf".to_string()
    } else {
        format!("{value:>5.1}")
    }
}

#[cfg(feature = "effect-meter-cli")]
fn linear_to_db(value: f32) -> f32 {
    if value <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * value.log10()
    }
}

#[cfg(feature = "effect-meter-cli")]
fn format_delta_db(before: f32, after: f32) -> String {
    if before.is_finite() && after.is_finite() {
        format!("{:+4.1}", after - before)
    } else {
        "n/a".to_string()
    }
}

#[cfg(feature = "effect-meter-cli")]
fn truncate_ascii(value: &str, width: usize) -> String {
    spectral_graph::truncate_ascii(value, width)
}

#[cfg(feature = "effect-meter-cli")]
fn effect_rows_len(
    effect_names: &[String],
    effect_levels: Option<&[EffectLevelSnapshot]>,
    effect_spectral: Option<&[Option<EffectBandSnapshot>]>,
    width: u16,
) -> usize {
    let rows = effect_names
        .len()
        .max(effect_levels.map_or(0, <[EffectLevelSnapshot]>::len))
        .max(effect_spectral.map_or(0, <[Option<EffectBandSnapshot>]>::len))
        .min(spectral_graph::EFFECT_METER_MAX_VISIBLE_EFFECTS);
    if !spectral_graph::live_spectral_graph_visible(effect_names, width) {
        return rows;
    }
    let spectral_rows = effect_names
        .iter()
        .take(rows)
        .enumerate()
        .filter(|(index, name)| {
            effect_spectral
                .and_then(|snapshots| snapshots.get(*index))
                .is_some_and(Option::is_some)
                || spectral_graph::supports_spectral_graph(name)
        })
        .count();
    rows + spectral_rows
}

/// Render the TUI frame for container info.
pub fn draw_info(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    info: &Info,
    file_path: &str,
) {
    let _ = terminal.draw(|f| {
        let title_text = format!("{}\nv{}", title_banner(), env!("CARGO_PKG_VERSION"));
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
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Track Durations"),
            );
        f.render_widget(tracks, chunks[2]);

        let footer = Paragraph::new("Press q, Esc, or Enter to exit")
            .style(Style::default().fg(Color::Blue));
        f.render_widget(footer, chunks[3]);
    });
}

#[cfg(test)]
mod tests {
    use super::title_banner;
    #[cfg(feature = "effect-meter-cli")]
    use super::{effect_meter_text, EffectLevelSnapshot};
    #[cfg(feature = "output-meter")]
    use super::{format_db, render_bar};
    #[cfg(feature = "effect-meter-cli-spectral")]
    use proteus_lib::dsp::meter::{BandLevels, EffectBandSnapshot};

    #[test]
    fn title_banner_is_non_empty() {
        assert!(!title_banner().trim().is_empty());
    }

    #[cfg(feature = "output-meter")]
    #[test]
    fn render_bar_respects_width() {
        let bar = render_bar(0.5, 10);
        assert_eq!(bar.chars().count(), 10);
    }

    #[cfg(feature = "output-meter")]
    #[test]
    fn format_db_handles_negative_infinity() {
        assert_eq!(format_db(f32::NEG_INFINITY), "-inf");
    }

    #[cfg(feature = "effect-meter-cli")]
    #[test]
    fn effect_meter_text_includes_effect_names_and_before_after_levels() {
        let names = vec!["Gain".to_string(), "LowPassFilter".to_string()];
        let snapshots = vec![
            EffectLevelSnapshot {
                input: proteus_lib::dsp::meter::LevelSnapshot {
                    peak: vec![0.25, 0.2],
                    rms: vec![0.1, 0.1],
                },
                output: proteus_lib::dsp::meter::LevelSnapshot {
                    peak: vec![0.5, 0.4],
                    rms: vec![0.2, 0.2],
                },
            },
            EffectLevelSnapshot {
                input: proteus_lib::dsp::meter::LevelSnapshot {
                    peak: vec![0.5, 0.4],
                    rms: vec![0.2, 0.2],
                },
                output: proteus_lib::dsp::meter::LevelSnapshot {
                    peak: vec![0.2, 0.15],
                    rms: vec![0.1, 0.1],
                },
            },
        ];

        let text = effect_meter_text(&names, Some(&snapshots), None, 100);
        let rendered = text
            .lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("[0] Gain"));
        assert!(rendered.contains("[1] LowPassFilter"));
        assert!(rendered.contains("in ["));
        assert!(rendered.contains("out ["));
    }

    #[cfg(feature = "effect-meter-cli-spectral")]
    #[test]
    fn effect_meter_text_renders_spectral_row_for_supported_effects_on_wide_layouts() {
        let names = vec!["LowPassFilter".to_string()];
        let snapshots = vec![EffectLevelSnapshot {
            input: proteus_lib::dsp::meter::LevelSnapshot {
                peak: vec![0.5, 0.4],
                rms: vec![0.2, 0.2],
            },
            output: proteus_lib::dsp::meter::LevelSnapshot {
                peak: vec![0.2, 0.15],
                rms: vec![0.1, 0.1],
            },
        }];
        let spectral = vec![Some(EffectBandSnapshot {
            input: BandLevels {
                bands_db: vec![-10.0, -10.0],
                band_centers_hz: vec![400.0, 4_000.0],
            },
            output: BandLevels {
                bands_db: vec![-10.0, -24.0],
                band_centers_hz: vec![400.0, 4_000.0],
            },
        })];

        let text = effect_meter_text(&names, Some(&snapshots), Some(&spectral), 100);
        let rendered = text
            .lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("spec d:"));
    }

    #[cfg(feature = "effect-meter-cli-spectral")]
    #[test]
    fn effect_meter_text_hides_spectral_row_on_narrow_layouts() {
        let names = vec!["HighPassFilter".to_string()];
        let snapshots = vec![EffectLevelSnapshot {
            input: proteus_lib::dsp::meter::LevelSnapshot {
                peak: vec![0.5, 0.4],
                rms: vec![0.2, 0.2],
            },
            output: proteus_lib::dsp::meter::LevelSnapshot {
                peak: vec![0.2, 0.15],
                rms: vec![0.1, 0.1],
            },
        }];

        let text = effect_meter_text(&names, Some(&snapshots), None, 50);
        let rendered = text
            .lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!rendered.contains("spec d:"));
    }
}
