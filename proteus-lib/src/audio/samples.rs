//! Helpers for working with `rodio` sample buffers.

use rodio::{buffer::SamplesBuffer, Source};

/// Clone a [`SamplesBuffer`] into two independent buffers.
///
/// `rodio::SamplesBuffer` owns its backing Vec; this helper extracts the data
/// and builds two new buffers with identical content.
///
/// # Example
/// ```rust
/// use rodio::buffer::SamplesBuffer;
/// use proteus_lib::audio::samples::clone_samples_buffer;
///
/// let buffer = SamplesBuffer::new(2, 48_000, vec![0.0f32; 4]);
/// let (a, b) = clone_samples_buffer(buffer);
/// assert_eq!(a.count(), b.count());
/// ```
pub fn clone_samples_buffer(buffer: SamplesBuffer) -> (SamplesBuffer, SamplesBuffer) {
    let sample_rate = buffer.sample_rate();
    let buffered = buffer.buffered();
    let vector_samples = buffered.clone().collect::<Vec<f32>>();
    let clone1 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples.clone());
    let clone2 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples);

    (clone1, clone2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_samples_buffer_preserves_metadata_and_samples() {
        let source = SamplesBuffer::new(2, 48_000, vec![0.1_f32, -0.1, 0.2, -0.2]);
        let (left, right) = clone_samples_buffer(source);

        assert_eq!(left.channels(), 2);
        assert_eq!(right.channels(), 2);
        assert_eq!(left.sample_rate(), 48_000);
        assert_eq!(right.sample_rate(), 48_000);

        let left_samples: Vec<f32> = left.buffered().collect();
        let right_samples: Vec<f32> = right.buffered().collect();
        assert_eq!(left_samples, right_samples);
    }

    #[test]
    fn clone_samples_buffer_preserves_frame_count() {
        let source = SamplesBuffer::new(1, 44_100, vec![0.1_f32, 0.2, 0.3]);
        let (left, right) = clone_samples_buffer(source);
        let left_count = left.count();
        let right_count = right.count();
        assert_eq!(left_count, right_count);
        assert_eq!(left_count, 3);
    }
}
