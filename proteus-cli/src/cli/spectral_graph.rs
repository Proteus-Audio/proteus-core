//! Compact ASCII spectral graph helpers shared by CLI views.

use proteus_lib::dsp::meter::EffectBandSnapshot;

pub(crate) const EFFECT_METER_MIN_WIDTH: u16 = 48;
pub(crate) const EFFECT_METER_SPECTRAL_MIN_WIDTH: u16 = 60;
pub(crate) const EFFECT_METER_MAX_VISIBLE_EFFECTS: usize = 6;

const MIN_GRAPH_WIDTH: usize = 16;
const MAX_GRAPH_WIDTH: usize = 24;
const GRAPH_NOISE_FLOOR_DB: f32 = -72.0;
const GRAPH_DYNAMIC_RANGE_DB: f32 = 36.0;
const GRAPH_ABSOLUTE_RANGE_DB: f32 = 72.0;

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

pub(crate) fn render_output_graph(snapshot: &EffectBandSnapshot, width: usize) -> String {
    let intensities = spectral_output_intensities(snapshot);
    if intensities.is_empty() {
        return placeholder_graph(width, "no data");
    }
    if intensities.iter().all(|value| *value <= 0.0) {
        return placeholder_graph(width, "quiet");
    }

    let width = graph_width_from_available(width);
    let mut graph = String::with_capacity(width);
    for column in 0..width {
        let position = if width <= 1 {
            0.0
        } else {
            column as f32 * (intensities.len() - 1) as f32 / (width - 1) as f32
        };
        let left = position.floor() as usize;
        let right = position.ceil() as usize;
        let frac = position - left as f32;
        let intensity = intensities[left] * (1.0 - frac) + intensities[right] * frac;
        graph.push(output_glyph(intensity));
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

fn spectral_output_intensities(snapshot: &EffectBandSnapshot) -> Vec<f32> {
    let max_output_db = snapshot
        .output
        .bands_db
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .max_by(|left, right| left.total_cmp(right))
        .unwrap_or(f32::NEG_INFINITY);
    if !max_output_db.is_finite() || max_output_db < GRAPH_NOISE_FLOOR_DB {
        return Vec::new();
    }

    let overall_strength =
        ((max_output_db - GRAPH_NOISE_FLOOR_DB) / GRAPH_ABSOLUTE_RANGE_DB).clamp(0.0, 1.0);
    snapshot
        .output
        .bands_db
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let output_db = if output.is_finite() {
                *output
            } else {
                f32::NEG_INFINITY
            };
            let input_db = snapshot
                .input
                .bands_db
                .get(index)
                .copied()
                .filter(|value| value.is_finite())
                .unwrap_or(output_db);
            let band_signal_db = input_db.max(output_db);
            if !band_signal_db.is_finite() || band_signal_db < GRAPH_NOISE_FLOOR_DB {
                0.0
            } else {
                let shape_strength = ((output_db - (max_output_db - GRAPH_DYNAMIC_RANGE_DB))
                    / GRAPH_DYNAMIC_RANGE_DB)
                    .clamp(0.0, 1.0);
                shape_strength * overall_strength
            }
        })
        .collect()
}

fn output_glyph(intensity: f32) -> char {
    let magnitude = (intensity.clamp(0.0, 1.0) * 4.0).round() as usize;
    ['.', '-', '~', '=', '#'][magnitude]
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
    fn low_pass_like_output_graph_fades_high_end() {
        let graph =
            render_output_graph(&snapshot(&[-10.0, -10.0, -10.0], &[-8.0, -18.0, -42.0]), 24);
        assert!(matches!(graph.chars().next(), Some('#' | '=')));
        assert!(matches!(graph.chars().last(), Some('.' | '-')));
    }

    #[test]
    fn high_pass_like_output_graph_fades_low_end() {
        let graph =
            render_output_graph(&snapshot(&[-10.0, -10.0, -10.0], &[-42.0, -18.0, -8.0]), 24);
        assert!(matches!(graph.chars().next(), Some('.' | '-')));
        assert!(matches!(graph.chars().last(), Some('#' | '=')));
    }

    #[test]
    fn weak_output_graph_is_dimmer_than_strong_output_graph() {
        let strong = render_output_graph(&snapshot(&[-8.0, -8.0, -8.0], &[-8.0, -20.0, -44.0]), 24);
        let weak = render_output_graph(
            &snapshot(&[-48.0, -48.0, -48.0], &[-48.0, -60.0, -84.0]),
            24,
        );

        let strong_weight = graph_weight(&strong);
        let weak_weight = graph_weight(&weak);
        assert!(strong_weight > weak_weight);
    }

    #[test]
    fn placeholder_graph_is_bounded() {
        let graph = placeholder_graph(12, "warming up");
        assert_eq!(graph.chars().count(), 16);
    }

    fn graph_weight(graph: &str) -> usize {
        graph
            .chars()
            .map(|ch| match ch {
                '.' => 0,
                '-' => 1,
                '~' => 2,
                '=' => 3,
                '#' => 4,
                _ => 0,
            })
            .sum()
    }
}
