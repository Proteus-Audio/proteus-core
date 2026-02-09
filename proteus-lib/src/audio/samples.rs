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
    let vector_samples = buffered.clone().into_iter().collect::<Vec<f32>>();
    let clone1 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples.clone());
    let clone2 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples);

    (clone1, clone2)
}
