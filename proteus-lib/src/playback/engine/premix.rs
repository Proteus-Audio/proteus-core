//! FIFO queue for premixed interleaved samples before DSP processing.

/// Buffered premix stream that decouples track mixing from DSP chunking.
///
/// Uses explicit head/tail indices over a contiguous `Vec<f32>` for O(1)
/// amortized front-consumption. Samples are appended at the tail and read
/// from the head without per-pop element shifting.
#[derive(Clone, Default)]
pub struct PremixBuffer {
    buf: Vec<f32>,
    head: usize,
    tail: usize,
}

impl std::fmt::Debug for PremixBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PremixBuffer")
            .field("len", &self.len())
            .field("capacity", &self.buf.len())
            .finish()
    }
}

impl PremixBuffer {
    /// Create an empty premix buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the number of queued interleaved samples.
    pub fn len(&self) -> usize {
        self.tail - self.head
    }

    /// Return `true` when no premixed samples are buffered.
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Append interleaved samples to the queue.
    ///
    /// # Arguments
    ///
    /// * `samples` - Premixed interleaved samples.
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        let needed = self.tail + samples.len();
        if needed > self.buf.len() {
            self.compact();
            let needed_after_compact = self.tail + samples.len();
            if needed_after_compact > self.buf.len() {
                self.buf.resize(needed_after_compact, 0.0);
            }
        }
        self.buf[self.tail..self.tail + samples.len()].copy_from_slice(samples);
        self.tail += samples.len();
    }

    /// Pop up to `sample_count` interleaved samples from the front.
    ///
    /// # Arguments
    ///
    /// * `sample_count` - Maximum number of samples to dequeue.
    ///
    /// # Returns
    ///
    /// A contiguous vector of dequeued samples, preserving order.
    pub fn pop_chunk(&mut self, sample_count: usize) -> Vec<f32> {
        let take = sample_count.min(self.len());
        let mut out = vec![0.0_f32; take];
        self.pop_chunk_into(&mut out);
        out
    }

    /// Pop up to `out.len()` interleaved samples into a caller-provided slice.
    ///
    /// # Arguments
    ///
    /// * `out` - Destination slice to fill with dequeued samples.
    ///
    /// # Returns
    ///
    /// The number of samples actually written into `out`.
    pub fn pop_chunk_into(&mut self, out: &mut [f32]) -> usize {
        let take = out.len().min(self.len());
        out[..take].copy_from_slice(&self.buf[self.head..self.head + take]);
        self.head += take;
        if self.head >= self.len() {
            self.compact();
        }
        take
    }

    /// Shift live data to the front of the backing buffer, reclaiming consumed
    /// head space without allocating.
    fn compact(&mut self) {
        if self.head == 0 {
            return;
        }
        let live = self.tail - self.head;
        self.buf.copy_within(self.head..self.tail, 0);
        self.head = 0;
        self.tail = live;
    }
}

#[cfg(test)]
mod tests {
    use super::PremixBuffer;

    #[test]
    fn push_pop_preserves_order() {
        let mut buf = PremixBuffer::new();
        buf.push_interleaved(&[0.1, 0.2, 0.3]);
        assert_eq!(buf.pop_chunk(2), vec![0.1, 0.2]);
        assert_eq!(buf.pop_chunk(2), vec![0.3]);
    }

    #[test]
    fn pop_returns_partial_when_fewer_available() {
        let mut buf = PremixBuffer::new();
        buf.push_interleaved(&[1.0]);
        let chunk = buf.pop_chunk(4);
        assert_eq!(chunk, vec![1.0]);
    }

    #[test]
    fn pop_empty_returns_empty() {
        let mut buf = PremixBuffer::new();
        assert_eq!(buf.pop_chunk(10), Vec::<f32>::new());
    }

    #[test]
    fn repeated_push_pop_cycles_compact_correctly() {
        let mut buf = PremixBuffer::new();
        for i in 0..20 {
            let base = i as f32 * 4.0;
            buf.push_interleaved(&[base, base + 1.0, base + 2.0, base + 3.0]);
            let chunk = buf.pop_chunk(4);
            assert_eq!(chunk, vec![base, base + 1.0, base + 2.0, base + 3.0]);
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn staggered_push_pop_preserves_ordering() {
        let mut buf = PremixBuffer::new();
        buf.push_interleaved(&[1.0, 2.0, 3.0]);
        buf.push_interleaved(&[4.0, 5.0]);
        assert_eq!(buf.pop_chunk(3), vec![1.0, 2.0, 3.0]);
        buf.push_interleaved(&[6.0, 7.0]);
        assert_eq!(buf.pop_chunk(4), vec![4.0, 5.0, 6.0, 7.0]);
        assert!(buf.is_empty());
    }

    #[test]
    fn pop_chunk_into_fills_caller_slice() {
        let mut buf = PremixBuffer::new();
        buf.push_interleaved(&[10.0, 20.0, 30.0]);
        let mut out = [0.0_f32; 2];
        let written = buf.pop_chunk_into(&mut out);
        assert_eq!(written, 2);
        assert_eq!(out, [10.0, 20.0]);
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn pop_chunk_into_partial_fill() {
        let mut buf = PremixBuffer::new();
        buf.push_interleaved(&[5.0]);
        let mut out = [0.0_f32; 4];
        let written = buf.pop_chunk_into(&mut out);
        assert_eq!(written, 1);
        assert_eq!(out[0], 5.0);
        assert!(buf.is_empty());
    }

    #[test]
    fn large_cycle_triggers_compaction_without_unbounded_growth() {
        let mut buf = PremixBuffer::new();
        let chunk: Vec<f32> = (0..256).map(|i| i as f32).collect();
        for _ in 0..100 {
            buf.push_interleaved(&chunk);
            let out = buf.pop_chunk(256);
            assert_eq!(out.len(), 256);
            assert_eq!(out[0], 0.0);
            assert_eq!(out[255], 255.0);
        }
        assert!(buf.is_empty());
        assert!(buf.buf.len() <= 512, "buffer should not grow unbounded");
    }
}
