#[cfg(not(feature = "real-fft"))]
mod complex_fft {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use rustfft::{num_complex::Complex, Fft, FftPlanner};

    // Taken from https://github.com/BordenJardine/reverb_vst

    #[derive(Clone)]
    pub struct Convolver {
        pub fft_size: usize,
        ir_segments: Vec<Vec<Complex<f32>>>,
        previous_frame_q: VecDeque<Vec<Complex<f32>>>,
        pub previous_tail: Vec<f32>,
        pending_output: Vec<f32>,
        fft_processor: Arc<dyn Fft<f32>>,
        ifft_processor: Arc<dyn Fft<f32>>,
    }

    impl Convolver {
        pub fn new(ir_signal: &[f32], fft_size: usize) -> Self {
            let mut planner = FftPlanner::<f32>::new();
            let fft_processor = planner.plan_fft_forward(fft_size);
            let ifft_processor = planner.plan_fft_inverse(fft_size);

            let ir_segments = segment_buffer(ir_signal, fft_size, &fft_processor);
            let segment_count = ir_segments.len();
            Self {
                fft_size,
                ir_segments,
                fft_processor,
                ifft_processor,
                previous_frame_q: init_previous_frame_q(segment_count, fft_size),
                previous_tail: init_previous_tail(fft_size / 2),
                pending_output: Vec::new(),
            }
        }

        pub fn process(&mut self, input_buffer: &[f32]) -> Vec<f32> {
            let io_len = input_buffer.len();
            let segment_size = self.fft_size / 2;
            let input_segments = segment_buffer(input_buffer, self.fft_size, &self.fft_processor);

            let mut output: Vec<f32> = Vec::with_capacity(io_len);
            let norm = self.fft_size as f32;

            if !self.pending_output.is_empty() {
                let take = io_len.min(self.pending_output.len());
                output.extend_from_slice(&self.pending_output[..take]);
                self.pending_output.drain(0..take);
            }

            for segment in input_segments {
                self.previous_frame_q.push_front(segment);
                self.previous_frame_q.pop_back();

                let mut convolved = self.convolve_frame();
                self.ifft_processor.process(&mut convolved);

                let mut time_domain: Vec<f32> = Vec::with_capacity(self.fft_size);
                for sample in convolved {
                    time_domain.push(sample.re / norm);
                }

                for i in 0..segment_size {
                    if let Some(sample) = time_domain.get_mut(i) {
                        *sample += self.previous_tail[i];
                    }
                }

                self.previous_tail = time_domain[segment_size..self.fft_size].to_vec();
                let remaining = io_len.saturating_sub(output.len());
                if remaining == 0 {
                    self.pending_output
                        .extend_from_slice(&time_domain[0..segment_size]);
                    continue;
                }
                if remaining >= segment_size {
                    output.extend_from_slice(&time_domain[0..segment_size]);
                } else {
                    output.extend_from_slice(&time_domain[0..remaining]);
                    self.pending_output
                        .extend_from_slice(&time_domain[remaining..segment_size]);
                }
            }

            output
        }

        fn convolve_frame(&mut self) -> Vec<Complex<f32>> {
            let mut convolved = vec![Complex { re: 0.0, im: 0.0 }; self.fft_size];

            for i in 0..self.ir_segments.len() {
                add_frames(
                    &mut convolved,
                    mult_frames(&self.previous_frame_q[i], &self.ir_segments[i]),
                );
            }
            convolved
        }
    }

    pub fn add_frames(f1: &mut [Complex<f32>], f2: Vec<Complex<f32>>) {
        for (sample1, sample2) in f1.iter_mut().zip(f2) {
            sample1.re = sample1.re + sample2.re;
            sample1.im = sample1.im + sample2.im;
        }
    }

    pub fn mult_frames(f1: &[Complex<f32>], f2: &[Complex<f32>]) -> Vec<Complex<f32>> {
        let mut out: Vec<Complex<f32>> = Vec::new();
        for (sample1, sample2) in f1.iter().zip(f2) {
            out.push(Complex {
                re: (sample1.re * sample2.re) - (sample1.im * sample2.im),
                im: (sample1.im * sample2.re) + (sample1.re * sample2.im),
            });
        }
        out
    }

    pub fn init_previous_tail(size: usize) -> Vec<f32> {
        let mut tail = Vec::new();
        for _ in 0..size {
            tail.push(0.0);
        }
        tail
    }

    pub fn segment_buffer(
        buffer: &[f32],
        fft_size: usize,
        fft_processor: &Arc<dyn Fft<f32>>,
    ) -> Vec<Vec<Complex<f32>>> {
        let mut segments = Vec::new();
        let segment_size = fft_size / 2;

        let mut index = 0;
        while index < buffer.len() {
            let mut new_segment: Vec<Complex<f32>> = Vec::new();
            for i in index..index + segment_size {
                match buffer.get(i) {
                    Some(sample) => new_segment.push(Complex { re: *sample, im: 0.0 }),
                    None => continue,
                }
            }
            while new_segment.len() < fft_size {
                new_segment.push(Complex { re: 0.0, im: 0.0 });
            }
            fft_processor.process(&mut new_segment);
            segments.push(new_segment);
            index += segment_size;
        }

        segments
    }

    pub fn init_previous_frame_q(
        segment_count: usize,
        fft_size: usize,
    ) -> VecDeque<Vec<Complex<f32>>> {
        let mut q = VecDeque::new();
        for _ in 0..segment_count {
            let mut empty = Vec::new();
            for _ in 0..fft_size {
                empty.push(Complex { re: 0.0, im: 0.0 });
            }
            q.push_back(empty);
        }
        q
    }
}

#[cfg(feature = "real-fft")]
mod real_fft {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use rustfft::num_complex::Complex;
    use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

    #[derive(Clone)]
    pub struct Convolver {
        pub fft_size: usize,
        ir_segments: Vec<Vec<Complex<f32>>>,
        previous_frame_q: VecDeque<Vec<Complex<f32>>>,
        pub previous_tail: Vec<f32>,
        pending_output: Vec<f32>,
        r2c: Arc<dyn RealToComplex<f32>>,
        c2r: Arc<dyn ComplexToReal<f32>>,
    }

    impl Convolver {
        pub fn new(ir_signal: &[f32], fft_size: usize) -> Self {
            let mut planner = RealFftPlanner::<f32>::new();
            let r2c = planner.plan_fft_forward(fft_size);
            let c2r = planner.plan_fft_inverse(fft_size);
            let spectrum_len = (fft_size / 2) + 1;

            let ir_segments = segment_buffer(ir_signal, fft_size, &r2c, spectrum_len);
            let segment_count = ir_segments.len();
            Self {
                fft_size,
                ir_segments,
                r2c,
                c2r,
                previous_frame_q: init_previous_frame_q(segment_count, spectrum_len),
                previous_tail: init_previous_tail(fft_size / 2),
                pending_output: Vec::new(),
            }
        }

        pub fn process(&mut self, input_buffer: &[f32]) -> Vec<f32> {
            let io_len = input_buffer.len();
            let segment_size = self.fft_size / 2;
            let spectrum_len = self.ir_segments.first().map(|seg| seg.len()).unwrap_or(0);
            let input_segments =
                segment_buffer(input_buffer, self.fft_size, &self.r2c, spectrum_len);

            let mut output: Vec<f32> = Vec::with_capacity(io_len);
            let norm = self.fft_size as f32;
            let spectrum_len = self.ir_segments.first().map(|seg| seg.len()).unwrap_or(0);

            if !self.pending_output.is_empty() {
                let take = io_len.min(self.pending_output.len());
                output.extend_from_slice(&self.pending_output[..take]);
                self.pending_output.drain(0..take);
            }

            for segment in input_segments {
                self.previous_frame_q.push_front(segment);
                self.previous_frame_q.pop_back();

                let mut convolved = vec![Complex { re: 0.0, im: 0.0 }; spectrum_len];
                for i in 0..self.ir_segments.len() {
                    add_frames(
                        &mut convolved,
                        mult_frames(&self.previous_frame_q[i], &self.ir_segments[i]),
                    );
                }

                let mut time_domain = vec![0.0_f32; self.fft_size];
                self.c2r
                    .process(&mut convolved, &mut time_domain)
                    .expect("real IFFT failed");

                for sample in &mut time_domain {
                    *sample /= norm;
                }

                for i in 0..segment_size {
                    time_domain[i] += self.previous_tail[i];
                }

                self.previous_tail = time_domain[segment_size..self.fft_size].to_vec();
                let remaining = io_len.saturating_sub(output.len());
                if remaining == 0 {
                    self.pending_output
                        .extend_from_slice(&time_domain[0..segment_size]);
                    continue;
                }
                if remaining >= segment_size {
                    output.extend_from_slice(&time_domain[0..segment_size]);
                } else {
                    output.extend_from_slice(&time_domain[0..remaining]);
                    self.pending_output
                        .extend_from_slice(&time_domain[remaining..segment_size]);
                }
            }

            output
        }
    }

    fn add_frames(f1: &mut [Complex<f32>], f2: Vec<Complex<f32>>) {
        for (sample1, sample2) in f1.iter_mut().zip(f2) {
            sample1.re = sample1.re + sample2.re;
            sample1.im = sample1.im + sample2.im;
        }
    }

    fn mult_frames(f1: &[Complex<f32>], f2: &[Complex<f32>]) -> Vec<Complex<f32>> {
        let mut out: Vec<Complex<f32>> = Vec::with_capacity(f1.len());
        for (sample1, sample2) in f1.iter().zip(f2) {
            out.push(Complex {
                re: (sample1.re * sample2.re) - (sample1.im * sample2.im),
                im: (sample1.im * sample2.re) + (sample1.re * sample2.im),
            });
        }
        out
    }

    fn init_previous_tail(size: usize) -> Vec<f32> {
        vec![0.0; size]
    }

    fn segment_buffer(
        buffer: &[f32],
        fft_size: usize,
        r2c: &Arc<dyn RealToComplex<f32>>,
        spectrum_len: usize,
    ) -> Vec<Vec<Complex<f32>>> {
        let mut segments = Vec::new();
        let segment_size = fft_size / 2;

        let mut index = 0;
        while index < buffer.len() {
            let mut time_domain = vec![0.0_f32; fft_size];
            for (offset, sample) in buffer
                .iter()
                .skip(index)
                .take(segment_size)
                .enumerate()
            {
                time_domain[offset] = *sample;
            }

            let mut spectrum = vec![Complex { re: 0.0, im: 0.0 }; spectrum_len];
            r2c
                .process(&mut time_domain, &mut spectrum)
                .expect("real FFT failed");
            segments.push(spectrum);
            index += segment_size;
        }

        segments
    }

    fn init_previous_frame_q(
        segment_count: usize,
        spectrum_len: usize,
    ) -> VecDeque<Vec<Complex<f32>>> {
        let mut q = VecDeque::new();
        for _ in 0..segment_count {
            q.push_back(vec![Complex { re: 0.0, im: 0.0 }; spectrum_len]);
        }
        q
    }
}

#[cfg(not(feature = "real-fft"))]
pub use complex_fft::Convolver;

#[cfg(feature = "real-fft")]
pub use real_fft::Convolver;
