use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::sync::OnceLock;
use text_to_ascii_art::{fonts, to_art};

use crate::controls::StatusSnapshot;
use crate::logging::{LogKind, LogLine};

/// Render the TUI frame (title, controls, status, logs).
pub fn draw_status(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    status: &StatusSnapshot,
    log_lines: &[LogLine],
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

        let status_widget = Paragraph::new(status.text.as_str())
            .style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL).title("Playback"));
        f.render_widget(status_widget, chunks[2]);

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
