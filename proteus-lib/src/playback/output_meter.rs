//! Output meter for tracking playback levels.

#[cfg(feature = "output-meter")]
mod enabled {
    use std::collections::VecDeque;

    use rodio::buffer::SamplesBuffer;
    use rodio::Source;

    #[derive(Debug)]
    pub struct OutputMeter {
        channels: usize,
        levels: Vec<f32>,
        queue: VecDeque<Vec<f32>>,
        offset: usize,
    }

    impl OutputMeter {
        pub fn new(channels: usize) -> Self {
            let channels = channels.max(1);
            Self {
                channels,
                levels: vec![0.0; channels],
                queue: VecDeque::new(),
                offset: 0,
            }
        }

        pub fn reset(&mut self) {
            self.queue.clear();
            self.offset = 0;
            self.levels.fill(0.0);
        }

        pub fn push_samples(&mut self, buffer: &SamplesBuffer) {
            let channels = buffer.channels().max(1) as usize;
            if channels != self.channels {
                self.channels = channels;
                self.levels = vec![0.0; channels];
            }

            let mut peaks = vec![0.0_f32; channels];
            for (idx, sample) in buffer.clone().enumerate() {
                let ch = idx % channels;
                let value = sample.abs();
                if value > peaks[ch] {
                    peaks[ch] = value;
                }
            }
            self.queue.push_back(peaks);
        }

        pub fn update_from_sink_len(&mut self, sink_len: usize) {
            let total_enqueued = self.offset + self.queue.len();
            if total_enqueued == 0 {
                return;
            }

            let mut current_index = total_enqueued.saturating_sub(sink_len);
            if current_index >= total_enqueued {
                current_index = total_enqueued - 1;
            }

            let keep_from = current_index.saturating_sub(2);
            while self.offset < keep_from && !self.queue.is_empty() {
                self.queue.pop_front();
                self.offset += 1;
            }

            let idx = current_index.saturating_sub(self.offset);
            if let Some(frame) = self.queue.get(idx) {
                if self.levels.len() != frame.len() {
                    self.levels = frame.clone();
                    self.channels = self.levels.len().max(1);
                } else {
                    self.levels.copy_from_slice(frame);
                }
            }
        }

        pub fn levels(&self) -> Vec<f32> {
            self.levels.clone()
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
        pub fn new(channels: usize) -> Self {
            Self {
                channels: channels.max(1),
            }
        }

        pub fn reset(&mut self) {}

        pub fn push_samples(&mut self, _buffer: &SamplesBuffer) {}

        pub fn update_from_sink_len(&mut self, _sink_len: usize) {}

        pub fn levels(&self) -> Vec<f32> {
            vec![0.0; self.channels]
        }
    }
}

#[cfg(not(feature = "output-meter"))]
pub use disabled::OutputMeter;
#[cfg(feature = "output-meter")]
pub use enabled::OutputMeter;
