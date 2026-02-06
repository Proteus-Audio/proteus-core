use rodio::{buffer::SamplesBuffer, Source};

pub fn clone_samples_buffer(buffer: SamplesBuffer) -> (SamplesBuffer, SamplesBuffer) {
    let sample_rate = buffer.sample_rate();
    let buffered = buffer.buffered();
    let vector_samples = buffered.clone().into_iter().collect::<Vec<f32>>();
    let clone1 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples.clone());
    let clone2 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples);
    (clone1, clone2)
}
