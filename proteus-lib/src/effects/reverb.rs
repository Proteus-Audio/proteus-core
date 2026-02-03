use std::time::Instant;

use log::debug;
use rodio::{
    buffer::SamplesBuffer,
    dynamic_mixer::{self, DynamicMixer},
    Source,
};

use crate::effects::convolution::Convolver;
use crate::effects::impulse_response::ImpulseResponse;
use crate::effects::spring_impulse_response::SPRING_IMPULSE_RESPONSE;

const FFT_SIZE: usize = 24576;
// const FFT_SIZE: usize = 4096;
// const FFT_SIZE: usize = 1024;

pub struct Reverb {
    channels: usize,
    dry_wet: f32,
    convolvers: Vec<Convolver>,
}

impl Reverb {
    pub fn new(channels: usize, dry_wet: f32) -> Self {
        let mut convolvers = Vec::with_capacity(channels);
        for _ in 0..channels {
            convolvers.push(Convolver::new(SPRING_IMPULSE_RESPONSE, FFT_SIZE));
        }

        Self {
            channels,
            dry_wet,
            convolvers,
        }
    }

    pub fn new_with_impulse_response(
        channels: usize,
        dry_wet: f32,
        impulse_response: &ImpulseResponse,
    ) -> Self {
        let mut convolvers = Vec::with_capacity(channels);
        for channel_index in 0..channels {
            let ir_channel = impulse_response.channel_for_output(channel_index);
            convolvers.push(Convolver::new(ir_channel, FFT_SIZE));
        }

        Self {
            channels,
            dry_wet,
            convolvers,
        }
    }

    pub fn process(&mut self, input_buffer: &[f32]) -> Vec<f32> {
        let channels = self.split_channels(input_buffer);
        let mut processed = Vec::with_capacity(self.channels);
        for (i, channel) in channels.iter().enumerate() {
            let processed_channel = self.process_channel(channel, i);
            processed.push(processed_channel);
        }
        let mixed = self.mix_channels(&channels, &processed);
        mixed
    }

    pub fn process_mixer(&mut self, mixer: DynamicMixer<f32>) -> SamplesBuffer<f32> {
        let sample_rate = mixer.sample_rate();
        let mixer_buffered = mixer.buffered();
        let vector_samples = mixer_buffered.clone().into_iter().collect::<Vec<f32>>();
        let processed = self.process(&vector_samples);
        SamplesBuffer::new(mixer_buffered.channels(), sample_rate, processed)
    }

    fn process_channel(&mut self, channel: &[f32], index: usize) -> Vec<f32> {
        let start = Instant::now();
        let convolver = &mut self.convolvers[index];
        // convolver.fft_size = channel.len();
        // let mut convolver = Convolver::new(SPRING_IMPULSE_RESPONSE, FFT_SIZE);

        debug!("Convolver fft size: {:?}", convolver.fft_size);
        debug!("Channel length: {:?}", channel.len());

        let time_to_create_convolver = Instant::now();
        debug!(
            "Time taken to create convolver #{}: {:?}",
            index,
            time_to_create_convolver.duration_since(start)
        );
        // println!("Channel length: {:?}", channel.len());
        // println!("Previous tail length: {:?}", self.previous_tails.len());
        // convolver.previous_tail = if self.previous_tails.len() > index {
        //     self.previous_tails[index].clone()
        // } else {
        //     self.previous_tails.push(vec![0.0; channel.len()]);
        //     self.previous_tails[index].clone()
        // };
        let processed = convolver.process(channel);
        // self.previous_tails[index] = convolver.previous_tail;
        let end = Instant::now();
        debug!(
            "Time taken to process channel #{}: {:?}",
            index,
            end.duration_since(start)
        );
        processed
    }

    fn split_channels(&self, input_buffer: &[f32]) -> Vec<Vec<f32>> {
        let mut unzipped = Vec::with_capacity(self.channels);
        for i in 0..self.channels {
            unzipped.push(
                input_buffer
                    .iter()
                    .skip(i)
                    .step_by(self.channels)
                    .cloned()
                    .collect::<Vec<f32>>(),
            );
        }
        unzipped
    }

    fn mix_channels(&self, dry: &Vec<Vec<f32>>, wet: &Vec<Vec<f32>>) -> Vec<f32> {
        let mut mixed = Vec::with_capacity(dry[0].len() * self.channels);
        let dry_amount = 1.0 - self.dry_wet;
        let wet_amount = self.dry_wet;

        // For each sample position
        for i in 0..dry[0].len() {
            // For each channel
            for ch in 0..self.channels {
                let mixed_sample = (dry[ch][i] * dry_amount) + (wet[ch][i] * wet_amount);
                mixed.push(mixed_sample);
            }
        }

        mixed
    }

    pub fn set_dry_wet(&mut self, dry_wet: f32) {
        self.dry_wet = dry_wet.clamp(0.0, 1.0);
    }

    pub fn clear_tail(&mut self) {
        for convolver in &mut self.convolvers {
            convolver.previous_tail.fill(0.0);
        }
    }
}

// pub fn apply_reverb(samples: Vec<f32>, dry_wet: f32) -> Vec<f32> {
//     // Clamp dry_wet between 0 and 1
//     let dry_wet = dry_wet.clamp(0.0, 1.0);
//     let dry_amount = 1.0 - dry_wet;
//     let wet_amount = dry_wet;

//     println!("Samples length: {:?}", samples.len());
//     let left_samples = samples.iter().step_by(2).cloned().collect::<Vec<f32>>();
//     let right_samples = samples
//         .iter()
//         .skip(1)
//         .step_by(2)
//         .cloned()
//         .collect::<Vec<f32>>();

//     let mut convolver_left = Convolver::new(SPRING_IMPULSE_RESPONSE, left_samples.len());
//     let mut convolver_right = Convolver::new(SPRING_IMPULSE_RESPONSE, right_samples.len());

//     let processed_left = convolver_left.process(&left_samples);
//     let processed_right = convolver_right.process(&right_samples);

//     let previous_tail_left = convolver_left.previous_tail;
//     let previous_tail_right = convolver_right.previous_tail;

//     println!("Previous tail left: {:?}", previous_tail_left.len());
//     println!("Previous tail right: {:?}", previous_tail_right.len());

//     // Mix dry and wet signals
//     let mut processed = Vec::with_capacity(processed_left.len() * 2);
//     for ((dry_l, dry_r), (wet_l, wet_r)) in left_samples.iter().zip(right_samples.iter())
//         .zip(processed_left.iter().zip(processed_right.iter()))
//     {
//         // Mix left channel
//         let mixed_l = (dry_l * dry_amount) + (wet_l * wet_amount);
//         // Mix right channel
//         let mixed_r = (dry_r * dry_amount) + (wet_r * wet_amount);

//         processed.push(mixed_l);
//         processed.push(mixed_r);
//     }

//     println!("Processed length: {:?}", processed.len());
//     processed
// }
