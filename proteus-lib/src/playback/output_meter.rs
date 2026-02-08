//! Output meter for tracking playback levels.

#[cfg(feature = "output-meter")]
mod enabled {
    use std::collections::VecDeque;

    use rodio::buffer::SamplesBuffer;
    use rodio::Source;

    #[derive(Debug)]
    struct Frame {
        peak: Vec<f32>,
        avg: Vec<f32>,
        len_samples: usize,
    }

    #[derive(Debug)]
    pub struct OutputMeter {
        sample_rate: u32,
        channels: usize,
        refresh_hz: f32,
        frame_samples_per_channel: usize,
        sample_remainder: f64,
        current_frame_remaining: usize,
        levels: Vec<f32>,
        averages: Vec<f32>,
        queue: VecDeque<Frame>,
    }

    impl OutputMeter {
        pub fn new(channels: usize, sample_rate: u32, refresh_hz: f32) -> Self {
            let channels = channels.max(1);
            let sample_rate = sample_rate.max(1);
            let refresh_hz = refresh_hz.max(1.0);
            Self {
                sample_rate,
                channels,
                refresh_hz,
                frame_samples_per_channel: frame_samples_per_channel(sample_rate, refresh_hz),
                sample_remainder: 0.0,
                current_frame_remaining: 0,
                levels: vec![0.0; channels],
                averages: vec![0.0; channels],
                queue: VecDeque::new(),
            }
        }

        pub fn reset(&mut self) {
            self.queue.clear();
            self.sample_remainder = 0.0;
            self.current_frame_remaining = 0;
            self.levels.fill(0.0);
            self.averages.fill(0.0);
        }

        pub fn set_refresh_hz(&mut self, refresh_hz: f32) {
            let refresh_hz = refresh_hz.max(1.0);
            if (refresh_hz - self.refresh_hz).abs() <= f32::EPSILON {
                return;
            }
            self.refresh_hz = refresh_hz;
            self.frame_samples_per_channel =
                frame_samples_per_channel(self.sample_rate, self.refresh_hz);
            self.reset();
        }

        pub fn push_samples(&mut self, buffer: &SamplesBuffer) {
            let channels = buffer.channels().max(1) as usize;
            let sample_rate = buffer.sample_rate().max(1);
            if channels != self.channels {
                self.channels = channels;
                self.levels = vec![0.0; channels];
                self.averages = vec![0.0; channels];
            }
            if sample_rate != self.sample_rate {
                self.sample_rate = sample_rate;
                self.frame_samples_per_channel =
                    frame_samples_per_channel(self.sample_rate, self.refresh_hz);
                self.reset();
            }

            let frame_len_samples = self.frame_samples_per_channel * channels;
            let mut peak = vec![0.0_f32; channels];
            let mut sum = vec![0.0_f32; channels];
            let mut count = vec![0_usize; channels];
            let mut in_frame = 0_usize;

            for (idx, sample) in buffer.clone().enumerate() {
                let ch = idx % channels;
                let value = sample.abs();
                if value > peak[ch] {
                    peak[ch] = value;
                }
                sum[ch] += value;
                count[ch] += 1;
                in_frame += 1;

                if in_frame >= frame_len_samples {
                    self.queue
                        .push_back(finalize_frame(&peak, &sum, &count, in_frame));
                    peak.fill(0.0);
                    sum.fill(0.0);
                    count.fill(0);
                    in_frame = 0;
                }
            }

            if in_frame > 0 {
                self.queue
                    .push_back(finalize_frame(&peak, &sum, &count, in_frame));
            }
        }

        pub fn advance(&mut self, elapsed_seconds: f64) {
            if elapsed_seconds <= 0.0 {
                return;
            }

            let mut samples = elapsed_seconds * self.sample_rate as f64 * self.channels as f64;
            samples += self.sample_remainder;
            let mut samples_to_advance = samples.floor() as usize;
            self.sample_remainder = samples - samples_to_advance as f64;

            while samples_to_advance > 0 {
                if self.current_frame_remaining == 0 {
                    let Some(frame) = self.queue.pop_front() else {
                        break;
                    };
                    self.levels = frame.peak;
                    self.averages = frame.avg;
                    self.current_frame_remaining = frame.len_samples;
                }

                let take = samples_to_advance.min(self.current_frame_remaining);
                self.current_frame_remaining -= take;
                samples_to_advance -= take;
            }
        }

        pub fn levels(&self) -> Vec<f32> {
            self.levels.clone()
        }

        pub fn averages(&self) -> Vec<f32> {
            self.averages.clone()
        }
    }

    fn frame_samples_per_channel(sample_rate: u32, refresh_hz: f32) -> usize {
        ((sample_rate as f32 / refresh_hz).round() as usize).max(1)
    }

    fn finalize_frame(peak: &[f32], sum: &[f32], count: &[usize], len_samples: usize) -> Frame {
        let mut avg = Vec::with_capacity(sum.len());
        for (idx, value) in sum.iter().enumerate() {
            let denom = count[idx].max(1) as f32;
            avg.push(value / denom);
        }
        Frame {
            peak: peak.to_vec(),
            avg,
            len_samples,
        }
    }
}

#[cfg(not(feature = "output-meter"))]
mod disabled {
    use rodio::buffer::SamplesBuffer;

    #[derive(Debug)]
    pub struct OutputMeter {
        channels: usize,
    }

    impl OutputMeter {
        pub fn new(channels: usize, _sample_rate: u32, _refresh_hz: f32) -> Self {
            Self {
                channels: channels.max(1),
            }
        }

        pub fn reset(&mut self) {}

        pub fn set_refresh_hz(&mut self, _refresh_hz: f32) {}

        pub fn push_samples(&mut self, _buffer: &SamplesBuffer) {}

        pub fn advance(&mut self, _elapsed_seconds: f64) {}

        pub fn levels(&self) -> Vec<f32> {
            vec![0.0; self.channels]
        }

        pub fn averages(&self) -> Vec<f32> {
            vec![0.0; self.channels]
        }
    }
}

#[cfg(feature = "output-meter")]
pub use enabled::OutputMeter;
#[cfg(not(feature = "output-meter"))]
pub use disabled::OutputMeter;
