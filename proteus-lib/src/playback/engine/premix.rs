//! FIFO queue for premixed interleaved samples before DSP processing.

use std::collections::VecDeque;

/// Buffered premix stream that decouples track mixing from DSP chunking.
///
/// The mixing stage appends interleaved samples into this queue, then the DSP
/// stage consumes fixed-size chunks to keep effect processing cadence stable.
#[derive(Debug, Default, Clone)]
pub struct PremixBuffer {
    samples: VecDeque<f32>,
}

impl PremixBuffer {
    /// Create an empty premix buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the number of queued interleaved samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Return `true` when no premixed samples are buffered.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Remove all buffered samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Append interleaved samples to the queue.
    ///
    /// # Arguments
    /// - `samples`: Premixed interleaved samples.
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        self.samples.extend(samples.iter().copied());
    }

    /// Pop up to `sample_count` interleaved samples from the front.
    ///
    /// # Arguments
    /// - `sample_count`: Maximum number of samples to dequeue.
    ///
    /// # Returns
    /// A contiguous vector of dequeued samples, preserving order.
    pub fn pop_chunk(&mut self, sample_count: usize) -> Vec<f32> {
        let take = sample_count.min(self.samples.len());
        self.samples.drain(0..take).collect()
    }
}
