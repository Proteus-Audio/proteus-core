#![cfg_attr(not(feature = "effect-meter-spectral"), allow(dead_code))]

//! Spectral analysis helpers used by runtime effect metering.

#[cfg(feature = "effect-meter-spectral")]
mod enabled {
    use std::sync::Arc;

    use realfft::{num_complex::Complex32, RealFftPlanner, RealToComplex};

    use crate::dsp::effects::{
        AudioEffect, HighEdgeFilterSettings, LowEdgeFilterSettings, MultibandEqEffect,
    };
    use crate::dsp::guardrails::sanitize_channels;
    use crate::dsp::meter::{BandLevels, EffectBandSnapshot};

    const FILTER_BUCKET_COUNT: usize = 12;
    const MIN_DB: f32 = -120.0;
    const MIN_POWER: f32 = 1.0e-12;

    #[derive(Debug, Clone, Copy)]
    struct Bucket {
        lower_hz: f32,
        upper_hz: f32,
        center_hz: f32,
    }

    #[derive(Debug)]
    struct SampleRing {
        channels: usize,
        frames: usize,
        samples: Vec<f32>,
        write_frame: usize,
        filled_frames: usize,
    }

    impl SampleRing {
        fn new(channels: usize, frames: usize) -> Self {
            let channels = sanitize_channels(channels);
            let frames = frames.max(1);
            Self {
                channels,
                frames,
                samples: vec![0.0; channels * frames],
                write_frame: 0,
                filled_frames: 0,
            }
        }

        fn push_interleaved(&mut self, samples: &[f32]) {
            if samples.is_empty() {
                return;
            }

            for frame in samples.chunks_exact(self.channels) {
                let start = self.write_frame * self.channels;
                self.samples[start..start + self.channels].copy_from_slice(frame);
                self.write_frame = (self.write_frame + 1) % self.frames;
                self.filled_frames = (self.filled_frames + 1).min(self.frames);
            }
        }

        fn write_windowed_channel(&self, channel: usize, window: &[f32], output: &mut [f32]) {
            output.fill(0.0);
            if self.filled_frames == 0 {
                return;
            }

            let pad_frames = self.frames - self.filled_frames;
            let oldest_frame = if self.filled_frames < self.frames {
                0
            } else {
                self.write_frame
            };

            for frame_index in 0..self.filled_frames {
                let source_frame = (oldest_frame + frame_index) % self.frames;
                let sample_index = source_frame * self.channels + channel;
                let output_index = pad_frames + frame_index;
                output[output_index] = self.samples[sample_index] * window[output_index];
            }
        }
    }

    /// Runtime spectral analyzer reused across refreshes.
    pub(crate) struct EffectSpectralAnalyzer {
        channels: usize,
        fft_frames: usize,
        fft_forward: Arc<dyn RealToComplex<f32>>,
        scratch: Vec<Complex32>,
        input_ring: SampleRing,
        output_ring: SampleRing,
        time_buffer: Vec<f32>,
        spectrum_buffer: Vec<Complex32>,
        summed_power: Vec<f32>,
        window: Vec<f32>,
    }

    impl EffectSpectralAnalyzer {
        pub(crate) fn new(channels: usize, fft_frames: usize) -> Self {
            let channels = sanitize_channels(channels);
            let fft_frames = fft_frames.max(1);
            let mut planner = RealFftPlanner::<f32>::new();
            let fft_forward = planner.plan_fft_forward(fft_frames);
            let spectrum_len = fft_forward.complex_len();
            Self {
                channels,
                fft_frames,
                scratch: fft_forward.make_scratch_vec(),
                input_ring: SampleRing::new(channels, fft_frames),
                output_ring: SampleRing::new(channels, fft_frames),
                time_buffer: fft_forward.make_input_vec(),
                spectrum_buffer: fft_forward.make_output_vec(),
                summed_power: vec![0.0; spectrum_len],
                window: hann_window(fft_frames),
                fft_forward,
            }
        }

        pub(crate) fn capture_input(&mut self, samples: &[f32]) {
            self.input_ring.push_interleaved(samples);
        }

        pub(crate) fn capture_output(&mut self, samples: &[f32]) {
            self.output_ring.push_interleaved(samples);
        }

        pub(crate) fn analyze(
            &mut self,
            effect: &AudioEffect,
            sample_rate: u32,
        ) -> Option<EffectBandSnapshot> {
            let buckets = buckets_for_effect(effect, sample_rate)?;
            Some(EffectBandSnapshot {
                input: self.analyze_input_direction(&buckets, sample_rate),
                output: self.analyze_output_direction(&buckets, sample_rate),
            })
        }

        fn analyze_input_direction(&mut self, buckets: &[Bucket], sample_rate: u32) -> BandLevels {
            self.analyze_direction(true, buckets, sample_rate)
        }

        fn analyze_output_direction(&mut self, buckets: &[Bucket], sample_rate: u32) -> BandLevels {
            self.analyze_direction(false, buckets, sample_rate)
        }

        fn analyze_direction(
            &mut self,
            use_input_ring: bool,
            buckets: &[Bucket],
            sample_rate: u32,
        ) -> BandLevels {
            if buckets.is_empty() {
                return BandLevels::default();
            }

            self.summed_power.fill(0.0);
            for channel in 0..self.channels {
                if use_input_ring {
                    self.input_ring.write_windowed_channel(
                        channel,
                        &self.window,
                        &mut self.time_buffer,
                    );
                } else {
                    self.output_ring.write_windowed_channel(
                        channel,
                        &self.window,
                        &mut self.time_buffer,
                    );
                }
                if self
                    .fft_forward
                    .process_with_scratch(
                        &mut self.time_buffer,
                        &mut self.spectrum_buffer,
                        &mut self.scratch,
                    )
                    .is_err()
                {
                    return BandLevels::default();
                }
                for (index, bin) in self.spectrum_buffer.iter().enumerate() {
                    self.summed_power[index] += bin.norm_sqr();
                }
            }

            reduce_buckets(
                &self.summed_power,
                self.fft_frames,
                sample_rate,
                buckets,
                self.channels,
            )
        }
    }

    pub(crate) fn relevant_effect(effect: &AudioEffect) -> bool {
        matches!(
            effect,
            AudioEffect::LowPassFilter(_)
                | AudioEffect::HighPassFilter(_)
                | AudioEffect::MultibandEq(_)
        )
    }

    fn reduce_buckets(
        power_bins: &[f32],
        fft_frames: usize,
        sample_rate: u32,
        buckets: &[Bucket],
        channels: usize,
    ) -> BandLevels {
        let bin_hz = sample_rate as f32 / fft_frames.max(1) as f32;
        let mut bands_db = Vec::with_capacity(buckets.len());
        let mut band_centers_hz = Vec::with_capacity(buckets.len());

        for bucket in buckets {
            let mut sum = 0.0_f32;
            let mut count = 0_usize;
            for (index, power) in power_bins.iter().copied().enumerate() {
                let freq_hz = index as f32 * bin_hz;
                if freq_hz >= bucket.lower_hz && freq_hz <= bucket.upper_hz {
                    sum += power;
                    count += 1;
                }
            }

            let avg_power = if count == 0 {
                MIN_POWER
            } else {
                sum / count as f32 / channels.max(1) as f32
            };
            bands_db.push((10.0 * avg_power.max(MIN_POWER).log10()).max(MIN_DB));
            band_centers_hz.push(bucket.center_hz);
        }

        BandLevels {
            bands_db,
            band_centers_hz,
        }
    }

    fn buckets_for_effect(effect: &AudioEffect, sample_rate: u32) -> Option<Vec<Bucket>> {
        let nyquist_hz = (sample_rate as f32 * 0.5).max(1.0);
        match effect {
            AudioEffect::LowPassFilter(_effect) => {
                Some(full_spectrum_buckets(nyquist_hz, FILTER_BUCKET_COUNT))
            }
            AudioEffect::HighPassFilter(_effect) => {
                Some(full_spectrum_buckets(nyquist_hz, FILTER_BUCKET_COUNT))
            }
            AudioEffect::MultibandEq(effect) => Some(multiband_eq_buckets(effect, nyquist_hz)),
            _ => None,
        }
    }

    fn full_spectrum_buckets(nyquist_hz: f32, count: usize) -> Vec<Bucket> {
        let count = count.max(1);
        if count == 1 {
            return vec![Bucket {
                lower_hz: 0.0,
                upper_hz: nyquist_hz,
                center_hz: (nyquist_hz * 0.5).max(1.0),
            }];
        }

        if nyquist_hz <= 40.0 {
            return linear_buckets(nyquist_hz, count);
        }

        let min_hz = 20.0_f32.min(nyquist_hz * 0.5).max(1.0);
        let ratio = nyquist_hz / min_hz;
        if !ratio.is_finite() || ratio <= 1.1 {
            return linear_buckets(nyquist_hz, count);
        }

        let mut boundaries = Vec::with_capacity(count + 1);
        boundaries.push(0.0);
        for index in 1..count {
            let t = index as f32 / count as f32;
            boundaries.push(min_hz * ratio.powf(t));
        }
        boundaries.push(nyquist_hz);

        boundaries
            .windows(2)
            .map(|pair| Bucket {
                lower_hz: pair[0],
                upper_hz: pair[1],
                center_hz: bucket_center_hz(pair[0], pair[1]),
            })
            .collect()
    }

    fn linear_buckets(nyquist_hz: f32, count: usize) -> Vec<Bucket> {
        (0..count)
            .map(|index| {
                let lower_hz = nyquist_hz * index as f32 / count as f32;
                let upper_hz = nyquist_hz * (index + 1) as f32 / count as f32;
                Bucket {
                    lower_hz,
                    upper_hz,
                    center_hz: bucket_center_hz(lower_hz, upper_hz),
                }
            })
            .collect()
    }

    fn bucket_center_hz(lower_hz: f32, upper_hz: f32) -> f32 {
        if lower_hz <= 0.0 {
            (upper_hz * 0.5).max(1.0)
        } else {
            (lower_hz * upper_hz).sqrt().max(1.0)
        }
    }

    fn multiband_eq_buckets(effect: &MultibandEqEffect, nyquist_hz: f32) -> Vec<Bucket> {
        let mut control_freqs = Vec::new();
        if let Some(low_edge) = effect.settings.low_edge.as_ref() {
            control_freqs.push(low_edge_frequency(low_edge));
        }
        control_freqs.extend(
            effect
                .settings
                .points
                .iter()
                .map(|point| point.freq_hz as f32),
        );
        if let Some(high_edge) = effect.settings.high_edge.as_ref() {
            control_freqs.push(high_edge_frequency(high_edge));
        }
        control_freqs.sort_by(|left, right| left.total_cmp(right));
        control_freqs.dedup_by(|left, right| (*left - *right).abs() <= f32::EPSILON);

        if control_freqs.is_empty() {
            return Vec::new();
        }

        let mut boundaries = Vec::with_capacity(control_freqs.len() + 1);
        boundaries.push(0.0);
        for pair in control_freqs.windows(2) {
            boundaries.push((pair[0] + pair[1]) * 0.5);
        }
        boundaries.push(nyquist_hz);

        control_freqs
            .iter()
            .enumerate()
            .map(|(index, center_hz)| Bucket {
                lower_hz: boundaries[index],
                upper_hz: boundaries[index + 1],
                center_hz: *center_hz,
            })
            .collect()
    }

    fn low_edge_frequency(edge: &LowEdgeFilterSettings) -> f32 {
        match edge {
            LowEdgeFilterSettings::HighPass { freq_hz, .. }
            | LowEdgeFilterSettings::LowShelf { freq_hz, .. } => *freq_hz as f32,
        }
    }

    fn high_edge_frequency(edge: &HighEdgeFilterSettings) -> f32 {
        match edge {
            HighEdgeFilterSettings::LowPass { freq_hz, .. }
            | HighEdgeFilterSettings::HighShelf { freq_hz, .. } => *freq_hz as f32,
        }
    }

    fn hann_window(size: usize) -> Vec<f32> {
        if size <= 1 {
            return vec![1.0; size.max(1)];
        }
        (0..size)
            .map(|index| {
                0.5 - 0.5 * ((2.0 * std::f32::consts::PI * index as f32) / (size - 1) as f32).cos()
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use crate::dsp::effects::{
            AudioEffect, EqPointSettings, HighPassFilterEffect, LowPassFilterEffect,
            MultibandEqEffect,
        };

        use super::{relevant_effect, EffectSpectralAnalyzer};

        fn sine_wave(freq_hz: f32, sample_rate: u32, frames: usize) -> Vec<f32> {
            (0..frames)
                .flat_map(|frame| {
                    let phase =
                        2.0 * std::f32::consts::PI * freq_hz * frame as f32 / sample_rate as f32;
                    let sample = phase.sin();
                    [sample, sample]
                })
                .collect()
        }

        #[test]
        fn relevant_effect_matches_supported_filter_types() {
            assert!(relevant_effect(&AudioEffect::LowPassFilter(
                LowPassFilterEffect::default()
            )));
            assert!(relevant_effect(&AudioEffect::HighPassFilter(
                HighPassFilterEffect::default()
            )));
            assert!(relevant_effect(&AudioEffect::MultibandEq(
                MultibandEqEffect::default()
            )));
        }

        #[test]
        fn low_pass_buckets_capture_low_tone_energy() {
            let sample_rate = 48_000;
            let mut effect = LowPassFilterEffect::default();
            effect.settings.freq_hz = 1_000;
            let effect = AudioEffect::LowPassFilter(effect);
            let mut analyzer = EffectSpectralAnalyzer::new(2, 2048);
            let samples = sine_wave(200.0, sample_rate, 2048);
            analyzer.capture_input(&samples);
            analyzer.capture_output(&samples);

            let snapshot = analyzer.analyze(&effect, sample_rate).expect("snapshot");
            let max_index = snapshot
                .input
                .bands_db
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .expect("band");
            assert!(snapshot.input.band_centers_hz[max_index] < 500.0);
        }

        #[test]
        fn high_pass_buckets_capture_high_tone_energy() {
            let sample_rate = 48_000;
            let mut effect = HighPassFilterEffect::default();
            effect.settings.freq_hz = 1_000;
            let effect = AudioEffect::HighPassFilter(effect);
            let mut analyzer = EffectSpectralAnalyzer::new(2, 2048);
            let samples = sine_wave(5_000.0, sample_rate, 2048);
            analyzer.capture_input(&samples);
            analyzer.capture_output(&samples);

            let snapshot = analyzer.analyze(&effect, sample_rate).expect("snapshot");
            let max_index = snapshot
                .output
                .bands_db
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .expect("band");
            assert!(snapshot.output.band_centers_hz[max_index] > 2_000.0);
        }

        #[test]
        fn low_and_high_pass_use_dense_full_spectrum_buckets() {
            let sample_rate = 48_000;

            let low = super::buckets_for_effect(
                &AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
                sample_rate,
            )
            .expect("lowpass buckets");
            let high = super::buckets_for_effect(
                &AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
                sample_rate,
            )
            .expect("highpass buckets");

            assert_eq!(low.len(), super::FILTER_BUCKET_COUNT);
            assert_eq!(high.len(), super::FILTER_BUCKET_COUNT);
            assert!(low
                .windows(2)
                .all(|pair| pair[0].center_hz < pair[1].center_hz));
            assert!(high
                .windows(2)
                .all(|pair| pair[0].center_hz < pair[1].center_hz));
        }

        #[test]
        fn multiband_eq_buckets_align_to_control_frequencies() {
            let sample_rate = 48_000;
            let mut effect = MultibandEqEffect::default();
            effect.settings.points = vec![
                EqPointSettings::new(200, 0.8, 0.0),
                EqPointSettings::new(1_000, 0.8, 0.0),
                EqPointSettings::new(8_000, 0.8, 0.0),
            ];
            let effect = AudioEffect::MultibandEq(effect);
            let mut analyzer = EffectSpectralAnalyzer::new(2, 2048);
            let samples = sine_wave(1_000.0, sample_rate, 2048);
            analyzer.capture_input(&samples);
            analyzer.capture_output(&samples);

            let snapshot = analyzer.analyze(&effect, sample_rate).expect("snapshot");
            let max_index = snapshot
                .input
                .bands_db
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .expect("band");
            assert_eq!(snapshot.input.band_centers_hz[max_index].round(), 1_000.0);
        }
    }
}

#[cfg(feature = "effect-meter-spectral")]
pub(crate) use enabled::{relevant_effect, EffectSpectralAnalyzer};

#[cfg(not(feature = "effect-meter-spectral"))]
pub(crate) fn relevant_effect(_effect: &crate::dsp::effects::AudioEffect) -> bool {
    false
}

#[cfg(not(feature = "effect-meter-spectral"))]
pub(crate) struct EffectSpectralAnalyzer;

#[cfg(not(feature = "effect-meter-spectral"))]
impl EffectSpectralAnalyzer {
    pub(crate) fn new(_channels: usize, _fft_frames: usize) -> Self {
        Self
    }

    pub(crate) fn capture_input(&mut self, _samples: &[f32]) {}

    pub(crate) fn capture_output(&mut self, _samples: &[f32]) {}

    pub(crate) fn analyze(
        &mut self,
        _effect: &crate::dsp::effects::AudioEffect,
        _sample_rate: u32,
    ) -> Option<crate::dsp::meter::EffectBandSnapshot> {
        None
    }
}
