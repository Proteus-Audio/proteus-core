//! Public metering and analysis types for DSP inspection features.

#[cfg(feature = "effect-meter")]
pub(crate) mod frequency_response;
pub mod level;
pub(crate) mod spectral;

/// Per-channel time-domain level measurements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelSnapshot {
    /// Peak absolute value for each channel.
    pub peak: Vec<f32>,
    /// RMS value for each channel.
    pub rms: Vec<f32>,
}

/// Input/output level measurements captured around a single effect slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectLevelSnapshot {
    /// Levels measured immediately before the effect.
    pub input: LevelSnapshot,
    /// Levels measured immediately after the effect.
    pub output: LevelSnapshot,
}

/// A single point on an analytical frequency-response curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrequencyResponsePoint {
    /// Probe frequency in Hz.
    pub freq_hz: f32,
    /// Gain in dB at `freq_hz`.
    pub gain_db: f32,
}

/// Analytical frequency response for a filter-like effect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterResponseCurve {
    /// Composite response across all configured sections.
    pub composite: Vec<FrequencyResponsePoint>,
    /// Per-section response curves.
    ///
    /// Single-filter effects leave this empty.
    pub per_band: Vec<Vec<FrequencyResponsePoint>>,
}

/// Spectral energy values for a single measurement direction.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BandLevels {
    /// Energy per analysis bucket in dB.
    pub bands_db: Vec<f32>,
    /// Center-frequency labels for each bucket in Hz.
    pub band_centers_hz: Vec<f32>,
}

/// Spectral band levels measured before and after a filter effect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectBandSnapshot {
    /// Spectral bucket levels measured before the effect.
    pub input: BandLevels,
    /// Spectral bucket levels measured after the effect.
    pub output: BandLevels,
}
