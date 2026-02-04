use rand::Rng;

use crate::dsp::convolution::Convolver;

#[derive(Debug, Clone, Copy)]
pub struct DspBenchConfig {
    pub sample_rate: u32,
    pub input_seconds: f32,
    pub ir_seconds: f32,
    pub fft_size: usize,
    pub iterations: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct DspBenchResult {
    pub avg_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub audio_time_ms: f64,
    pub rt_factor: f64,
}

pub fn bench_convolver(config: DspBenchConfig) -> DspBenchResult {
    let input_len = (config.sample_rate as f32 * config.input_seconds).max(1.0) as usize;
    let ir_len = (config.sample_rate as f32 * config.ir_seconds).max(1.0) as usize;

    let mut rng = rand::thread_rng();
    let input: Vec<f32> = (0..input_len)
        .map(|_| rng.gen_range(-1.0_f32..1.0_f32))
        .collect();
    let ir: Vec<f32> = (0..ir_len)
        .map(|_| rng.gen_range(-1.0_f32..1.0_f32))
        .collect();

    let mut convolver = Convolver::new(&ir, config.fft_size);
    let mut times: Vec<f64> = Vec::with_capacity(config.iterations.max(1));

    for _ in 0..config.iterations.max(1) {
        let start = std::time::Instant::now();
        let _ = convolver.process(&input);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        times.push(elapsed);
    }

    let min_ms = times
        .iter()
        .copied()
        .fold(f64::INFINITY, |a, b| a.min(b));
    let max_ms = times.iter().copied().fold(0.0_f64, |a, b| a.max(b));
    let avg_ms = times.iter().sum::<f64>() / times.len() as f64;
    let audio_time_ms = (input_len as f64 / config.sample_rate as f64) * 1000.0;
    let rt_factor = if audio_time_ms > 0.0 {
        avg_ms / audio_time_ms
    } else {
        0.0
    };

    DspBenchResult {
        avg_ms,
        min_ms: if min_ms.is_finite() { min_ms } else { 0.0 },
        max_ms,
        audio_time_ms,
        rt_factor,
    }
}
