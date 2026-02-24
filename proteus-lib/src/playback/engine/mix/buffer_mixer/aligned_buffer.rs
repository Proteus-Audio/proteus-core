//! Internal aligned sample buffer representation.
//!
//! This stores explicit audio sample spans and virtual zero spans in order so the
//! mixer can preserve exact alignment semantics without materializing large zero
//! buffers during startup/seek.

use std::collections::VecDeque;

/// One ordered span in an instance buffer.
#[derive(Debug, Clone)]
pub(super) enum BufferSegment {
    /// A virtual run of silent samples.
    Zeros(usize),
    /// Owned real samples with a read cursor.
    Samples { data: Vec<f32>, pos: usize },
}

/// FIFO of ordered sample/zero segments for a single instance.
#[derive(Debug, Clone, Default)]
pub(super) struct AlignedSampleBuffer {
    segments: VecDeque<BufferSegment>,
    len_samples: usize,
}

impl AlignedSampleBuffer {
    pub(super) fn with_capacity(_capacity_samples: usize) -> Self {
        Self::default()
    }

    pub(super) fn len(&self) -> usize {
        self.len_samples
    }

    pub(super) fn pop_front(&mut self) -> Option<f32> {
        loop {
            let front = self.segments.front_mut()?;
            match front {
                BufferSegment::Zeros(count) => {
                    if *count == 0 {
                        self.segments.pop_front();
                        continue;
                    }
                    *count -= 1;
                    self.len_samples = self.len_samples.saturating_sub(1);
                    if *count == 0 {
                        self.segments.pop_front();
                    }
                    return Some(0.0);
                }
                BufferSegment::Samples { data, pos } => {
                    if *pos >= data.len() {
                        self.segments.pop_front();
                        continue;
                    }
                    let value = data[*pos];
                    *pos += 1;
                    self.len_samples = self.len_samples.saturating_sub(1);
                    if *pos >= data.len() {
                        self.segments.pop_front();
                    }
                    return Some(value);
                }
            }
        }
    }

    pub(super) fn push_zeros(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        match self.segments.back_mut() {
            Some(BufferSegment::Zeros(existing)) => {
                *existing = existing.saturating_add(count);
            }
            _ => self.segments.push_back(BufferSegment::Zeros(count)),
        }
        self.len_samples = self.len_samples.saturating_add(count);
    }

    pub(super) fn push_samples_from_slice(&mut self, slice: &[f32]) {
        if slice.is_empty() {
            return;
        }
        self.segments.push_back(BufferSegment::Samples {
            data: slice.to_vec(),
            pos: 0,
        });
        self.len_samples = self.len_samples.saturating_add(slice.len());
    }

    pub(super) fn push_owned_samples(&mut self, mut data: Vec<f32>) {
        if data.is_empty() {
            return;
        }
        if data.capacity() > data.len() {
            data.shrink_to_fit();
        }
        let len = data.len();
        self.segments
            .push_back(BufferSegment::Samples { data, pos: 0 });
        self.len_samples = self.len_samples.saturating_add(len);
    }
}
