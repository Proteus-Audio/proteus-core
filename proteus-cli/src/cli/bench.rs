//! Benchmark entry points for CLI DSP tests.

use std::{
    io::{self, IsTerminal},
    path::Path,
    time::{Duration, Instant},
};

use clap::ArgMatches;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::CODEC_TYPE_NULL,
    errors::{Error, Result},
};

use proteus_lib::{
    container::prot::Prot,
    diagnostics::audio_bench::{
        benchmark_audio_with_progress, AudioBenchmarkConfig, AudioBenchmarkReport,
    },
    dsp::effects::{convolution_reverb::ImpulseResponseSpec, AudioEffect},
    tools::decode::{get_decoder, get_reader},
};

use crate::logging;

/// Run the bench subcommand.
pub fn run_bench_subcommand(args: &ArgMatches) -> Result<i32> {
    match args.subcommand() {
        // Synthetic benchmarks are retained for compatibility with existing scripts.
        Some(("dsp", bench_args)) => run_single_bench(bench_args).map(|code| code.unwrap_or(0)),
        Some(("sweep", bench_args)) => run_sweep_bench(bench_args).map(|code| code.unwrap_or(0)),
        Some(_) => Ok(1),
        None if args.try_get_one::<String>("INPUT").ok().flatten().is_some() => {
            run_audio_benchmark(args)
        }
        None => Ok(1),
    }
}

/// Decode an input once, then benchmark its PCM independently of decode and I/O costs.
fn run_audio_benchmark(args: &ArgMatches) -> Result<i32> {
    let input_path = args
        .get_one::<String>("INPUT")
        .expect("bench input is present when this function is called");
    let config = match benchmark_config(args) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            return Ok(1);
        }
    };

    // Bench owns stderr with concise status updates. Library diagnostics remain
    // available as warnings/errors, but routine setup logs do not obscure them.
    let _log_guard = logging::suppress_info_stderr();
    eprintln!("Preparing benchmark input…");
    let prepared_at = Instant::now();
    let input = load_benchmark_input(input_path)?;
    let preparation_time = prepared_at.elapsed();
    let config = AudioBenchmarkConfig {
        sample_rate: input.sample_rate,
        channels: input.channels,
        ..config
    };
    let report = benchmark_audio_with_progress(
        &input.samples,
        config,
        input.container_path,
        input.impulse_response_spec,
        input.impulse_response_tail_db,
        input.project_effects,
        |case, total, name| eprintln!("Benchmarking ({case}/{total}) with {name}…"),
    )
    .map_err(|error| Error::IoError(io::Error::other(error)))?;

    if let Some(output_path) = args.get_one::<String>("output") {
        let markdown = markdown_report(&report, input_path, preparation_time);
        std::fs::write(output_path, markdown).map_err(Error::IoError)?;
        eprintln!("Saved Markdown report to {output_path}");
    } else {
        print!("{}", terminal_report(&report, input_path, preparation_time));
    }
    Ok(0)
}

fn markdown_report(
    report: &AudioBenchmarkReport,
    input_path: &str,
    preparation_time: Duration,
) -> String {
    format!(
        "# Input preparation\n\n\
         - Decoding and `.prot` settings load: {:.3} ms\n\
         - Audio source: first supported audio track\n\
         - The timing table below measures DSP processing of already-decoded PCM only.\n\n{}",
        milliseconds(preparation_time),
        report.to_markdown(input_path),
    )
}

fn terminal_report(
    report: &AudioBenchmarkReport,
    input_path: &str,
    preparation_time: Duration,
) -> String {
    render_terminal_report(
        report,
        input_path,
        preparation_time,
        io::stdout().is_terminal(),
    )
}

fn render_terminal_report(
    report: &AudioBenchmarkReport,
    input_path: &str,
    preparation_time: Duration,
    color: bool,
) -> String {
    let headers = [
        "Case",
        "Effects",
        "Total ms",
        "Average ms",
        "Min ms",
        "Max ms",
        "Real-time",
    ];
    let rows = report
        .cases
        .iter()
        .map(|case| {
            (
                vec![
                    case.name.clone(),
                    case.effect_count.to_string(),
                    format!("{:.3}", milliseconds(case.timing.total)),
                    format!("{:.3}", milliseconds(case.timing.average)),
                    format!("{:.3}", milliseconds(case.timing.minimum)),
                    format!("{:.3}", milliseconds(case.timing.maximum)),
                    format!("{:.3}x", case.timing.real_time_factor),
                ],
                case.timing.real_time_factor,
            )
        })
        .collect::<Vec<_>>();
    let mut widths = headers.map(str::len);
    for (row, _) in &rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    let separator = format!(
        "+-{}-+\n",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-")
    );
    let mut output = format!(
        "{}\n\
         Input: {}\n\
         PCM: {} frames, {} channels, {} Hz ({:.3} s)\n\
         Input preparation: {:.3} ms\n\
         Timed iterations: {}  •  Warm-up iterations: {}  •  Chunk: {} frames\n\n",
        style("Proteus audio benchmark", "1;36", color),
        input_path,
        report.frames,
        report.config.channels,
        report.config.sample_rate,
        report.audio_duration.as_secs_f64(),
        milliseconds(preparation_time),
        report.config.iterations.max(1),
        report.config.warmup_iterations,
        report.config.chunk_frames,
    );
    output.push_str(&separator);
    output.push_str("| ");
    for (index, header) in headers.iter().enumerate() {
        output.push_str(&style(header, "1;36", color));
        output.push_str(&format!(
            "{:width$} | ",
            "",
            width = widths[index] - header.len()
        ));
    }
    output.push('\n');
    output.push_str(&separator);
    for (row, real_time_factor) in rows {
        output.push_str("| ");
        for (index, value) in row.iter().enumerate() {
            let rendered = if index == 6 {
                style(
                    value,
                    if real_time_factor <= 1.0 { "32" } else { "33" },
                    color,
                )
            } else {
                value.clone()
            };
            if index == 0 {
                output.push_str(value);
                output.push_str(&format!(
                    "{:width$}",
                    "",
                    width = widths[index] - value.len()
                ));
            } else {
                output.push_str(&format!(
                    "{:>width$}",
                    "",
                    width = widths[index] - value.len()
                ));
                output.push_str(&rendered);
            }
            output.push_str(" | ");
        }
        output.push('\n');
    }
    output.push_str(&separator);
    output
}

fn style(value: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{value}\x1b[0m")
    } else {
        value.to_owned()
    }
}

fn benchmark_config(args: &ArgMatches) -> std::result::Result<AudioBenchmarkConfig, String> {
    Ok(AudioBenchmarkConfig {
        iterations: parse_positive(args, "iterations")?,
        warmup_iterations: parse_usize(args, "warmup-iterations")?,
        chunk_frames: parse_positive(args, "chunk-frames")?,
        ..AudioBenchmarkConfig::default()
    })
}

fn parse_positive(args: &ArgMatches, key: &str) -> std::result::Result<usize, String> {
    let value = parse_usize(args, key)?;
    if value == 0 {
        return Err(format!("--{key} must be at least 1."));
    }
    Ok(value)
}

fn parse_usize(args: &ArgMatches, key: &str) -> std::result::Result<usize, String> {
    args.get_one::<String>(key)
        .expect("benchmark arguments have defaults")
        .parse::<usize>()
        .map_err(|_| format!("--{key} must be a non-negative integer."))
}

struct BenchmarkInput {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: usize,
    container_path: Option<String>,
    impulse_response_spec: Option<ImpulseResponseSpec>,
    impulse_response_tail_db: f32,
    project_effects: Option<Vec<AudioEffect>>,
}

fn load_benchmark_input(input_path: &str) -> Result<BenchmarkInput> {
    let is_prot = Path::new(input_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("prot"));
    let project = if is_prot {
        Some(Prot::try_new(input_path).map_err(|error| Error::IoError(io::Error::other(error)))?)
    } else {
        None
    };

    let (samples, sample_rate, channels) = decode_interleaved(input_path)?;
    Ok(BenchmarkInput {
        samples,
        sample_rate,
        channels,
        container_path: project.as_ref().and_then(Prot::get_container_path),
        impulse_response_spec: project.as_ref().and_then(Prot::get_impulse_response_spec),
        impulse_response_tail_db: project
            .as_ref()
            .and_then(Prot::get_impulse_response_tail_db)
            .unwrap_or(-60.0),
        project_effects: project.and_then(|project| project.get_effects()),
    })
}

fn decode_interleaved(input_path: &str) -> Result<(Vec<f32>, u32, usize)> {
    let mut format =
        get_reader(input_path).map_err(|error| Error::IoError(io::Error::other(error)))?;
    let track_id = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .map(|track| track.id)
        .ok_or(Error::Unsupported("no supported audio tracks"))?;
    let mut decoder =
        get_decoder(format.as_ref()).map_err(|error| Error::IoError(io::Error::other(error)))?;
    let mut samples = Vec::new();
    let mut sample_rate = None;
    let mut channels = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // Like the normal decode path, recover from a corrupt packet when possible.
            Err(Error::DecodeError(_)) => continue,
            Err(Error::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error),
        };
        let spec = *decoded.spec();
        let decoded_rate = spec.rate;
        let decoded_channels = spec.channels.count();
        match (sample_rate, channels) {
            (Some(rate), Some(channel_count))
                if rate != decoded_rate || channel_count != decoded_channels =>
            {
                return Err(Error::Unsupported(
                    "audio stream format changed while decoding",
                ));
            }
            (Some(_), Some(_)) => {}
            (None, None) => {
                sample_rate = Some(decoded_rate);
                channels = Some(decoded_channels);
            }
            _ => unreachable!("sample rate and channels are initialized together"),
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }

    let sample_rate = sample_rate.ok_or(Error::Unsupported("input did not decode any audio"))?;
    let channels = channels.ok_or(Error::Unsupported("input did not decode any audio"))?;
    Ok((samples, sample_rate, channels))
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// Execute a single FFT benchmark run.
fn run_single_bench(_args: &ArgMatches) -> Result<Option<i32>> {
    // Single-FFT benchmark for quick comparisons.
    #[cfg(not(feature = "bench"))]
    {
        eprintln!("Benchmarking requires the `bench` feature.");
        Ok(Some(1))
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

        Ok(Some(0))
    }
}

/// Execute a sweep benchmark across multiple FFT sizes.
fn run_sweep_bench(_args: &ArgMatches) -> Result<Option<i32>> {
    // Sweep a fixed FFT-size list to find a performance sweet spot.
    #[cfg(not(feature = "bench"))]
    {
        eprintln!("Benchmarking requires the `bench` feature.");
        Ok(Some(1))
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

        let results = proteus_lib::diagnostics::bench::bench_convolver_sweep(base, &fft_sizes);
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

        Ok(Some(0))
    }
}

#[cfg(test)]
mod tests {
    use super::run_bench_subcommand;
    use clap::Command;

    #[test]
    fn bench_without_subcommand_returns_one() {
        let args = Command::new("bench").get_matches_from(["bench"]);
        let code = run_bench_subcommand(&args).expect("bench command should run");
        assert_eq!(code, 1);
    }

    #[test]
    fn bench_unknown_subcommand_returns_one() {
        let args = Command::new("bench")
            .subcommand(Command::new("noop"))
            .get_matches_from(["bench", "noop"]);
        let code = run_bench_subcommand(&args).expect("bench command should run");
        assert_eq!(code, 1);
    }
}
