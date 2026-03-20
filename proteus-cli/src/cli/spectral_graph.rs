//! Compact ASCII spectral graph helpers shared by CLI views.

use proteus_lib::dsp::meter::EffectBandSnapshot;

pub(crate) const EFFECT_METER_MIN_WIDTH: u16 = 48;
pub(crate) const EFFECT_METER_SPECTRAL_MIN_WIDTH: u16 = 60;
pub(crate) const EFFECT_METER_MAX_VISIBLE_EFFECTS: usize = 6;

const MIN_GRAPH_WIDTH: usize = 16;
const MAX_GRAPH_WIDTH: usize = 24;

pub(crate) fn supports_spectral_graph(effect_name: &str) -> bool {
    matches!(
        effect_name,
        "LowPassFilter" | "HighPassFilter" | "MultibandEq"
    )
}

pub(crate) fn live_effect_meter_visible(
    effect_names: &[String],
    level_slots: usize,
    width: u16,
) -> bool {
    width >= EFFECT_METER_MIN_WIDTH && effect_names.len().max(level_slots) > 0
}

pub(crate) fn live_spectral_graph_visible(effect_names: &[String], width: u16) -> bool {
    width >= EFFECT_METER_SPECTRAL_MIN_WIDTH
        && effect_names
            .iter()
            .any(|name| supports_spectral_graph(name))
}

pub(crate) fn live_graph_width(width: u16) -> Option<usize> {
    if width < EFFECT_METER_SPECTRAL_MIN_WIDTH {
        None
    } else {
        Some(graph_width_from_available(width.saturating_sub(12) as usize))
    }
}

pub(crate) fn render_delta_graph(snapshot: &EffectBandSnapshot, width: usize) -> String {
    let deltas = spectral_deltas(snapshot);
    if deltas.is_empty() {
        return placeholder_graph(width, "no data");
    }

    let width = graph_width_from_available(width);
    let mut graph = String::with_capacity(width);
    for column in 0..width {
        let position = if width <= 1 {
            0.0
        } else {
            column as f32 * (deltas.len() - 1) as f32 / (width - 1) as f32
        };
        let left = position.floor() as usize;
        let right = position.ceil() as usize;
        let frac = position - left as f32;
        let delta_db = deltas[left] * (1.0 - frac) + deltas[right] * frac;
        graph.push(delta_glyph(delta_db));
    }
    graph
}

pub(crate) fn placeholder_graph(width: usize, label: &str) -> String {
    let width = graph_width_from_available(width);
    if width == 0 {
        return String::new();
    }

    let label = truncate_ascii(label, width);
    if label.chars().count() >= width {
        return label;
    }

    let left_fill = (width - label.chars().count()) / 2;
    let right_fill = width - left_fill - label.chars().count();
    format!(
        "{}{}{}",
        ".".repeat(left_fill),
        label,
        ".".repeat(right_fill)
    )
}

pub(crate) fn truncate_ascii(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_string()
    } else {
        value
            .chars()
            .take(width.saturating_sub(3))
            .collect::<String>()
            + "..."
    }
}

fn graph_width_from_available(width: usize) -> usize {
    width.clamp(MIN_GRAPH_WIDTH, MAX_GRAPH_WIDTH)
}

fn spectral_deltas(snapshot: &EffectBandSnapshot) -> Vec<f32> {
    snapshot
        .input
        .bands_db
        .iter()
        .zip(snapshot.output.bands_db.iter())
        .map(|(input, output)| {
            let delta = *output - *input;
            if delta.is_finite() {
                delta
            } else {
                0.0
            }
        })
        .collect()
}

fn delta_glyph(delta_db: f32) -> char {
    let magnitude = (delta_db.abs() / 6.0).ceil().clamp(0.0, 4.0) as usize;
    if delta_db <= -1.5 {
        ['.', '-', '~', '=', 'X'][magnitude]
    } else if delta_db >= 1.5 {
        ['.', '+', '*', '%', '#'][magnitude]
    } else {
        '.'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_lib::dsp::meter::BandLevels;

    fn snapshot(input: &[f32], output: &[f32]) -> EffectBandSnapshot {
        EffectBandSnapshot {
            input: BandLevels {
                bands_db: input.to_vec(),
                band_centers_hz: vec![200.0, 1_000.0, 4_000.0],
            },
            output: BandLevels {
                bands_db: output.to_vec(),
                band_centers_hz: vec![200.0, 1_000.0, 4_000.0],
            },
        }
    }

    #[test]
    fn low_pass_like_delta_graph_cuts_high_end() {
        let graph = render_delta_graph(
            &snapshot(&[-10.0, -10.0, -10.0], &[-10.0, -16.0, -28.0]),
            24,
        );
        assert!(graph.starts_with('.'));
        assert!(graph.ends_with('X') || graph.ends_with('='));
    }

    #[test]
    fn high_pass_like_delta_graph_cuts_low_end() {
        let graph = render_delta_graph(
            &snapshot(&[-10.0, -10.0, -10.0], &[-28.0, -16.0, -10.0]),
            24,
        );
        assert!(graph.starts_with('X') || graph.starts_with('='));
        assert!(graph.ends_with('.'));
    }

    #[test]
    fn placeholder_graph_is_bounded() {
        let graph = placeholder_graph(12, "warming up");
        assert_eq!(graph.chars().count(), 16);
    }
}
