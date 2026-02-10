//! CLI argument definitions for `proteus-cli`.

use clap::{Arg, ArgAction, Command};

/// Build the CLI argument parser and command definitions.
pub fn build_cli() -> Command {
    fn with_input_arg(cmd: Command, required: bool) -> Command {
        cmd.arg(
            Arg::new("INPUT")
                .help("The input file path, or - to use standard input")
                .required(required)
                .index(1),
        )
    }

    fn with_bench_common_args(cmd: Command) -> Command {
        cmd.arg(
            Arg::new("bench-input-seconds")
                .long("bench-input-seconds")
                .value_name("SECONDS")
                .default_value("1.0")
                .help("Input length in seconds for DSP benchmark"),
        )
        .arg(
            Arg::new("bench-ir-seconds")
                .long("bench-ir-seconds")
                .value_name("SECONDS")
                .default_value("2.0")
                .help("Impulse response length in seconds for DSP benchmark"),
        )
        .arg(
            Arg::new("bench-iterations")
                .long("bench-iterations")
                .value_name("COUNT")
                .default_value("5")
                .help("Number of iterations for DSP benchmark"),
        )
    }

    // Build the CLI definition in one place to keep main.rs slim.
    Command::new("Prot Play")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Adam Howard <adam.thomas.howard@gmail.com>")
        .about("Play Prot audio")
        .arg_required_else_help(true)
        .arg(
            Arg::new("seek")
                .long("seek")
                .short('s')
                .value_name("TIME")
                .help("Seek to the given time in seconds")
                .conflicts_with_all(&["verify"]),
        )
        .arg(
            Arg::new("GAIN")
                .long("gain")
                .short('g')
                .value_name("GAIN")
                .default_value("70")
                .help("The playback gain"),
        )
        .arg(
            Arg::new("effects-json")
                .long("effects-json")
                .short('E')
                .alias("effects")
                .value_name("PATH")
                .help("Path to JSON file containing Vec<AudioEffect>"),
        )
        .arg(
            Arg::new("start-buffer-ms")
                .long("start-buffer-ms")
                .value_name("MS")
                .default_value("20")
                .help("Amount of audio (ms) to buffer before starting playback"),
        )
        .arg(
            Arg::new("start-sink-chunks")
                .long("start-sink-chunks")
                .value_name("CHUNKS")
                .default_value("3")
                .help("Minimum sink chunks queued before playback starts/resumes"),
        )
        .arg(
            Arg::new("startup-silence-ms")
                .long("startup-silence-ms")
                .value_name("MS")
                .default_value("0")
                .help("Silence pre-roll before playback starts"),
        )
        .arg(
            Arg::new("startup-fade-ms")
                .long("startup-fade-ms")
                .value_name("MS")
                .default_value("150")
                .help("Fade-in duration at playback start"),
        )
        .arg(
            Arg::new("append-jitter-log-ms")
                .long("append-jitter-log-ms")
                .value_name("MS")
                .default_value("0")
                .help("Log sink append jitter events above this threshold (ms)"),
        )
        .arg(
            Arg::new("effect-boundary-log")
                .long("effect-boundary-log")
                .action(ArgAction::SetTrue)
                .help("Log per-effect boundary discontinuities in the DSP chain"),
        )
        .arg(
            Arg::new("track-eos-ms")
                .long("track-eos-ms")
                .value_name("MS")
                .default_value("1000")
                .help("Heuristic end-of-track threshold in ms for container tracks"),
        )
        .arg(
            Arg::new("read-durations")
                .long("read-durations")
                .action(ArgAction::SetTrue)
                .help("Read track durations metadata, then exit"),
        )
        .arg(
            Arg::new("scan-durations")
                .long("scan-durations")
                .action(ArgAction::SetTrue)
                .help("Scan all packets to compute per-track durations, then exit"),
        )
        .arg(
            Arg::new("verify")
                .long("verify")
                .short('v')
                .help("Verify the decoded audio is valid during playback"),
        )
        .arg(
            Arg::new("no-progress")
                .long("no-progress")
                .help("Do not display playback progress"),
        )
        .arg(
            Arg::new("no-gapless")
                .long("no-gapless")
                .help("Disable gapless decoding and playback"),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(ArgAction::SetTrue)
                .help("Suppress all console output"),
        )
        .arg(Arg::new("debug").short('d').help("Show debug output"))
        .arg(
            Arg::new("INPUT")
                .help("The input file path, or - to use standard input")
                .required(false)
                .index(1),
        )
        .subcommand(
            Command::new("bench")
                .about("Run DSP benchmarks without starting playback")
                .subcommand(with_bench_common_args(
                    Command::new("dsp")
                        .about("Run a synthetic DSP benchmark and exit")
                        .arg(
                            Arg::new("bench-fft-size")
                                .long("bench-fft-size")
                                .value_name("SIZE")
                                .default_value("24576")
                                .help("FFT size for DSP benchmark"),
                        ),
                ))
                .subcommand(with_bench_common_args(
                    Command::new("sweep").about("Run a sweep over multiple FFT sizes and exit"),
                )),
        )
        .subcommand(
            Command::new("verify")
                .about("Probe or decode audio without starting playback")
                .subcommand(with_input_arg(
                    Command::new("probe").about("Only probe the input for metadata"),
                    true,
                ))
                .subcommand(with_input_arg(
                    Command::new("decode").about("Decode, but do not play the audio"),
                    true,
                ))
                .subcommand(with_input_arg(
                    Command::new("verify")
                        .about("Verify the decoded audio is valid, but do not play the audio"),
                    true,
                )),
        )
        .subcommand(with_input_arg(
            Command::new("info")
                .about("Display container info in a TUI")
                .arg(
                    Arg::new("print")
                        .long("print")
                        .action(ArgAction::SetTrue)
                        .help("Print info to stdout instead of opening the TUI"),
                ),
            true,
        ))
        .subcommand(with_input_arg(
            Command::new("peaks")
                .about("Output per-channel waveform peaks as JSON")
                .arg(
                    Arg::new("limited")
                        .long("limited")
                        .action(ArgAction::SetTrue)
                        .help("Only process a single channel"),
                ),
            true,
        ))
        .subcommand(
            Command::new("create")
                .about("Emit default JSON payloads")
                .subcommand(
                    Command::new("effects-json")
                        .about("Print a default Vec<AudioEffect> JSON payload"),
                ),
        )
}
