//! Analytical response helpers for multiband EQ settings.

use crate::dsp::meter::frequency_response::{build_log_spaced_curve, identity_curve, sum_curves};
use crate::dsp::meter::FilterResponseCurve;

use super::biquad::{
    high_pass_response_db, high_shelf_response_db, low_pass_response_db, low_shelf_response_db,
    peaking_response_db, HighEdgeParams, LowEdgeParams,
};
use super::{sanitize_high_edge, sanitize_low_edge, MultibandEqEffect};

impl MultibandEqEffect {
    pub(crate) fn frequency_response_curve(
        &self,
        sample_rate: u32,
        num_points: usize,
    ) -> FilterResponseCurve {
        let mut per_band = Vec::new();

        if let Some(low_edge) = self
            .settings
            .low_edge
            .as_ref()
            .map(|edge| sanitize_low_edge(edge, sample_rate))
        {
            per_band.push(low_edge_curve(sample_rate, num_points, low_edge));
        }

        per_band.extend(self.settings.points.iter().map(|point| {
            build_log_spaced_curve(sample_rate, num_points, |freq_hz| {
                peaking_response_db(sample_rate, point.freq_hz, point.q, point.gain_db, freq_hz)
            })
        }));

        if let Some(high_edge) = self
            .settings
            .high_edge
            .as_ref()
            .map(|edge| sanitize_high_edge(edge, sample_rate))
        {
            per_band.push(high_edge_curve(sample_rate, num_points, high_edge));
        }

        let composite = if per_band.is_empty() {
            identity_curve(sample_rate, num_points)
        } else {
            sum_curves(&per_band)
        };

        FilterResponseCurve {
            composite,
            per_band,
        }
    }
}

fn low_edge_curve(
    sample_rate: u32,
    num_points: usize,
    params: LowEdgeParams,
) -> Vec<crate::dsp::meter::FrequencyResponsePoint> {
    match params {
        LowEdgeParams::HighPass { freq_hz, q } => {
            build_log_spaced_curve(sample_rate, num_points, |probe_hz| {
                high_pass_response_db(sample_rate, freq_hz, q, probe_hz)
            })
        }
        LowEdgeParams::LowShelf {
            freq_hz,
            q,
            gain_db,
        } => build_log_spaced_curve(sample_rate, num_points, |probe_hz| {
            low_shelf_response_db(sample_rate, freq_hz, q, gain_db, probe_hz)
        }),
    }
}

fn high_edge_curve(
    sample_rate: u32,
    num_points: usize,
    params: HighEdgeParams,
) -> Vec<crate::dsp::meter::FrequencyResponsePoint> {
    match params {
        HighEdgeParams::LowPass { freq_hz, q } => {
            build_log_spaced_curve(sample_rate, num_points, |probe_hz| {
                low_pass_response_db(sample_rate, freq_hz, q, probe_hz)
            })
        }
        HighEdgeParams::HighShelf {
            freq_hz,
            q,
            gain_db,
        } => build_log_spaced_curve(sample_rate, num_points, |probe_hz| {
            high_shelf_response_db(sample_rate, freq_hz, q, gain_db, probe_hz)
        }),
    }
}

#[cfg(test)]
mod tests {
    use crate::dsp::effects::{
        EqPointSettings, HighEdgeFilterSettings, LowEdgeFilterSettings, MultibandEqEffect,
    };

    #[test]
    fn multiband_eq_frequency_response_exposes_composite_and_sections() {
        let mut effect = MultibandEqEffect::default();
        effect.settings.low_edge = Some(LowEdgeFilterSettings::HighPass {
            freq_hz: 60,
            q: 0.7,
        });
        effect.settings.points = vec![
            EqPointSettings::new(250, 0.8, 3.0),
            EqPointSettings::new(1_000, 1.0, -6.0),
        ];
        effect.settings.high_edge = Some(HighEdgeFilterSettings::LowPass {
            freq_hz: 12_000,
            q: 0.7,
        });

        let curve = effect.frequency_response_curve(48_000, 64);

        assert_eq!(curve.composite.len(), 64);
        assert_eq!(curve.per_band.len(), 4);
        assert!(curve
            .per_band
            .iter()
            .all(|section_curve| section_curve.len() == 64));
    }
}
