use clap::ArgMatches;
use symphonia::core::errors::Result;

pub fn maybe_run_bench(args: &ArgMatches) -> Result<Option<i32>> {
    // Dispatches benchmark sub-modes and returns an exit code if handled.
    if args.get_flag("bench-dsp") {
        return run_single_bench(args);
    }
    if args.get_flag("bench-sweep") {
        return run_sweep_bench(args);
    }
    Ok(None)
}

fn run_single_bench(_args: &ArgMatches) -> Result<Option<i32>> {
    // Single-FFT benchmark for quick comparisons.
    #[cfg(not(feature = "bench"))]
    {
        eprintln!("Benchmarking requires the `bench` feature.");
        return Ok(Some(1));
    }
    #[cfg(feature = "bench")]
    {
        let args = _args;
        let fft_size = args
            .get_one::<String>("bench-fft-size")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let input_seconds = args
            .get_one::<String>("bench-input-seconds")
            .unwrap()
            .parse::<f32>()
            .unwrap();
        let ir_seconds = args
            .get_one::<String>("bench-ir-seconds")
            .unwrap()
            .parse::<f32>()
            .unwrap();
        let iterations = args
            .get_one::<String>("bench-iterations")
            .unwrap()
            .parse::<usize>()
            .unwrap();

        let result = proteus_lib::diagnostics::bench::bench_convolver(
            proteus_lib::diagnostics::bench::DspBenchConfig {
                sample_rate: 44_100,
                input_seconds,
                ir_seconds,
                fft_size,
                iterations,
            },
        );

        println!(
            "DSP bench (fft={} input={}s ir={}s iters={}): avg {:.2}ms (min {:.2}ms max {:.2}ms), audio {:.2}ms, rt {:.2}x, ir_segments {}",
            fft_size,
            input_seconds,
            ir_seconds,
            iterations,
            result.avg_ms,
            result.min_ms,
            result.max_ms,
            result.audio_time_ms,
            result.rt_factor,
            result.ir_segments
        );

        return Ok(Some(0));
    }
}

fn run_sweep_bench(_args: &ArgMatches) -> Result<Option<i32>> {
    // Sweep a fixed FFT-size list to find a performance sweet spot.
    #[cfg(not(feature = "bench"))]
    {
        eprintln!("Benchmarking requires the `bench` feature.");
        return Ok(Some(1));
    }
    #[cfg(feature = "bench")]
    {
        let args = _args;
        let fft_sizes = [8192, 12288, 16384, 20480, 24576, 32768];
        let input_seconds = args
            .get_one::<String>("bench-input-seconds")
            .unwrap()
            .parse::<f32>()
            .unwrap();
        let ir_seconds = args
            .get_one::<String>("bench-ir-seconds")
            .unwrap()
            .parse::<f32>()
            .unwrap();
        let iterations = args
            .get_one::<String>("bench-iterations")
            .unwrap()
            .parse::<usize>()
            .unwrap();

        let base = proteus_lib::diagnostics::bench::DspBenchConfig {
            sample_rate: 44_100,
            input_seconds,
            ir_seconds,
            fft_size: fft_sizes[0],
            iterations,
        };

        let results = proteus_lib::diagnostics::bench::bench_convolver_sweep(&base, &fft_sizes);
        println!(
            "DSP sweep (input={}s ir={}s iters={})",
            input_seconds, ir_seconds, iterations
        );
        println!("fft_size | avg_ms | min_ms | max_ms | rt_x | ir_segments");
        for (fft_size, result) in results {
            println!(
                "{:>7} | {:>6.2} | {:>6.2} | {:>6.2} | {:>4.2} | {:>11}",
                fft_size,
                result.avg_ms,
                result.min_ms,
                result.max_ms,
                result.rt_factor,
                result.ir_segments
            );
        }

        return Ok(Some(0));
    }
}
