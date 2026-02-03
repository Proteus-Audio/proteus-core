use log::debug;
use rustfft::{num_complex::Complex, FftPlanner};
use std::io::BufReader;

use crate::effects::convolution::Convolver;
use crate::effects::spring_impulse_response::SPRING_IMPULSE_RESPONSE;
use rodio::{buffer::SamplesBuffer, Decoder, Source};
use symphonia::core::audio::SampleBuffer;

pub fn apply_convolution_reverb(input_signal: Vec<f32>) -> Vec<f32> {
    // Load your impulse response (IR) file
    // TODO: This should be done once, not every time the effect is applied
    let ir_signal = load_impulse_response(
        "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/Impulse Responses/IR.wav",
    );

    // Perform the convolution here
    // This is a placeholder for actual convolution logic
    let convolved_signal = fft_convolution(&input_signal, &ir_signal);

    convolved_signal

    // input_signal
}

pub fn load_impulse_response(file_path: &str) -> Vec<f32> {
    let file = BufReader::new(std::fs::File::open(file_path).unwrap());
    let mut source = Decoder::new(file).unwrap();

    // println!("Sample rate: {:?}", source.sample_rate());
    // Load the WAV file and return its samples as a Vec<f32>
    // This function needs to be implemented

    let mut samples: Vec<f32> = Vec::new();

    while let Some(sample) = source.next() {
        debug!("Impulse response sample: {:?}", sample);
        samples.push(sample as f32);
    }

    samples
}

pub fn convolution(_input_signal: &[f32], _ir_signal: &[f32]) -> Vec<f32> {
    // Implement the convolution algorithm
    // This could be direct convolution or FFT-based convolution

    Vec::new()
}

fn fft_convolution(input_signal: &[f32], ir_signal: &[f32]) -> Vec<f32> {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(input_signal.len() + ir_signal.len() - 1);
    let ifft = planner.plan_fft_inverse(input_signal.len() + ir_signal.len() - 1);

    let mut input_fft = input_signal
        .iter()
        .map(|&f| Complex::new(f, 0.0))
        .collect::<Vec<_>>();
    let mut ir_fft = ir_signal
        .iter()
        .map(|&f| Complex::new(f, 0.0))
        .collect::<Vec<_>>();

    // Zero-padding to the same length
    input_fft.resize(
        input_fft.len() + ir_signal.len() - 1,
        Complex::new(0.0, 0.0),
    );
    ir_fft.resize(
        ir_fft.len() + input_signal.len() - 1,
        Complex::new(0.0, 0.0),
    );

    // Perform FFT
    fft.process(&mut input_fft);
    fft.process(&mut ir_fft);

    // Multiply in frequency domain
    for (input, ir) in input_fft.iter_mut().zip(ir_fft.iter()) {
        *input = *input * *ir;
    }

    // Perform inverse FFT
    ifft.process(&mut input_fft);

    // Normalize and extract real part
    input_fft
        .clone()
        .into_iter()
        .map(|c| c.re / input_fft.len() as f32)
        .collect()
}

pub fn simple_reverb(samples: Vec<f32>, delay_samples: usize, decay: f32) -> Vec<f32> {
    let mut processed = Vec::with_capacity(samples.len() + delay_samples);
    for i in 0..samples.len() {
        let delayed_index = i.checked_sub(delay_samples);
        let delayed_sample = delayed_index
            .and_then(|index| samples.get(index))
            .unwrap_or(&0.0)
            * decay;
        let current_sample = samples[i] + delayed_sample;
        processed.push(current_sample);
    }
    processed
}

pub fn apply_reverb(samples: Vec<f32>, dry_wet: f32) -> Vec<f32> {
    // Clamp dry_wet between 0 and 1
    let dry_wet = dry_wet.clamp(0.0, 1.0);
    let dry_amount = 1.0 - dry_wet;
    let wet_amount = dry_wet;

    debug!("Samples length: {:?}", samples.len());
    let left_samples = samples.iter().step_by(2).cloned().collect::<Vec<f32>>();
    let right_samples = samples
        .iter()
        .skip(1)
        .step_by(2)
        .cloned()
        .collect::<Vec<f32>>();
    
    let mut convolver_left = Convolver::new(SPRING_IMPULSE_RESPONSE, left_samples.len());
    let mut convolver_right = Convolver::new(SPRING_IMPULSE_RESPONSE, right_samples.len());

    let processed_left = convolver_left.process(&left_samples);
    let processed_right = convolver_right.process(&right_samples);

    let previous_tail_left = convolver_left.previous_tail;
    let previous_tail_right = convolver_right.previous_tail;

    debug!("Previous tail left: {:?}", previous_tail_left.len());
    debug!("Previous tail right: {:?}", previous_tail_right.len());

    // Mix dry and wet signals
    let mut processed = Vec::with_capacity(processed_left.len() * 2);
    for ((dry_l, dry_r), (wet_l, wet_r)) in left_samples.iter().zip(right_samples.iter())
        .zip(processed_left.iter().zip(processed_right.iter())) 
    {
        // Mix left channel
        let mixed_l = (dry_l * dry_amount) + (wet_l * wet_amount);
        // Mix right channel
        let mixed_r = (dry_r * dry_amount) + (wet_r * wet_amount);
        
        processed.push(mixed_l);
        processed.push(mixed_r);
    }
    
    debug!("Processed length: {:?}", processed.len());
    processed
}

// pub fn clone_sample_buffer(buffer: &SamplesBuffer<f32>) -> SamplesBuffer<f32> {
//     let sample_rate = buffer.sample_rate();
//     let cloned = buffer.copied();
//     let buffered = buffer.buffered();
//     let vector_samples = buffered.clone().into_iter().collect::<Vec<f32>>();
//     let cloned = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples);
//     cloned
// }
// fn clone_samples_buffer(buffer: &SamplesBuffer<f32>) -> SamplesBuffer<f32> {
//     // Extract the properties of the original buffer
//     let channels = buffer.channels();
//     let sample_rate = buffer.sample_rate();
//     let samples = buffer.clone().collect(); // Collect the samples into a Vec<f32>
//     // let samples: Vec<f32> = buffer.clone().collect(); // Collect the samples into a Vec<f32>

//     // Create a new SamplesBuffer with the same properties and samples
//     // SamplesBuffer::new(channels, sample_rate, samples)
//     samples
// }

pub fn clone_samples_buffer(
    buffer: SamplesBuffer<f32>,
) -> (SamplesBuffer<f32>, SamplesBuffer<f32>) {
    let sample_rate = buffer.sample_rate();
    let buffered = buffer.buffered();
    let vector_samples = buffered.clone().into_iter().collect::<Vec<f32>>();
    let clone1 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples.clone());
    let clone2 = SamplesBuffer::new(buffered.channels(), sample_rate, vector_samples);
    (clone1, clone2)
}
