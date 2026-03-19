//! CLI argument definitions for `proteus-cli`.

use clap::{Arg, ArgAction, Command};

fn with_input_arg(cmd: Command, required: bool) -> Command {
    cmd.arg(
        Arg::new("INPUT")
            .help("Input .prot/.mka file, audio file, or directory of nested audio files")
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

fn build_bench_subcommand() -> Command {
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
        ))
}

fn build_verify_subcommand() -> Command {
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
        ))
}

fn build_info_subcommand() -> Command {
    with_input_arg(
        Command::new("info")
            .about("Display container info in a TUI")
            .arg(
                Arg::new("print")
                    .long("print")
                    .action(ArgAction::SetTrue)
                    .help("Print info to stdout instead of opening the TUI"),
            ),
        true,
    )
}

fn build_peaks_subcommand() -> Command {
    Command::new("peaks")
        .about("Extract, write, and read waveform peaks")
        .arg(
            Arg::new("INPUT")
                .help("Legacy mode: input audio file path for JSON peak output")
                .required(false)
                .index(1),
        )
        .arg(
            Arg::new("limited")
                .long("limited")
                .action(ArgAction::SetTrue)
                .help("Only process a single channel"),
        )
        .subcommand(with_input_arg(
            Command::new("json")
                .about("Decode audio and output per-channel waveform peaks as JSON")
                .arg(
                    Arg::new("limited")
                        .long("limited")
                        .action(ArgAction::SetTrue)
                        .help("Only process a single channel"),
                ),
            true,
        ))
        .subcommand(
            Command::new("write")
                .about("Decode audio and write a binary peaks file")
                .arg(
                    Arg::new("INPUT")
                        .help("Input audio file path")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("OUTPUT")
                        .help("Output binary peaks file path")
                        .required(true)
                        .index(2),
                ),
        )
        .subcommand(with_input_arg(
            Command::new("read")
                .about("Read a binary peaks file and output JSON")
                .arg(
                    Arg::new("start")
                        .long("start")
                        .value_name("SECONDS")
                        .help("Start timestamp in seconds (requires --end)"),
                )
                .arg(
                    Arg::new("end")
                        .long("end")
                        .value_name("SECONDS")
                        .help("End timestamp in seconds (requires --start)"),
                )
                .arg(
                    Arg::new("peaks")
                        .long("peaks")
                        .value_name("COUNT")
                        .help("Target number of peaks to return per channel"),
                )
                .arg(
                    Arg::new("channels")
                        .long("channels")
                        .value_name("COUNT")
                        .help("Maximum number of channels to return"),
                ),
            true,
        ))
}

fn build_init_subcommand() -> Command {
    Command::new("init")
        .about("Generate shuffle/effects JSON for a directory of nested audio files")
        .arg(
            Arg::new("INPUT")
                .help("Directory containing nested audio files")
                .required(true)
                .index(1),
        )
}

fn build_create_subcommand() -> Command {
    Command::new("create")
        .about("Emit default JSON payloads")
        .subcommand(
            Command::new("effects-json").about("Print a default Vec<AudioEffect> JSON payload"),
        )
}

fn build_meter_subcommand() -> Command {
    Command::new("meter")
        .about("Offline DSP metering and effect-chain inspection")
        .subcommand(with_input_arg(
            Command::new("effects")
                .about("Run an input through the effects chain and print before/after metering")
                .arg(
                    Arg::new("effects-json")
                        .long("effects-json")
                        .short('E')
                        .alias("effects")
                        .value_name("PATH")
                        .help("Path to JSON file containing Vec<AudioEffect>"),
                )
                .arg(
                    Arg::new("seek")
                        .long("seek")
                        .short('s')
                        .value_name("TIME")
                        .default_value("0")
                        .help("Seek to the given time in seconds before metering"),
                )
                .arg(
                    Arg::new("duration")
                        .long("duration")
                        .value_name("SECONDS")
                        .help("Only meter this many seconds after the seek point"),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .default_value("table")
                        .help("Output format: table, bars, or json"),
                )
                .arg(
                    Arg::new("summary")
                        .long("summary")
                        .value_name("MODE")
                        .default_value("max")
                        .help("Summary mode: final or max"),
                )
                .arg(
                    Arg::new("spectral")
                        .long("spectral")
                        .action(ArgAction::SetTrue)
                        .help("Append spectral buckets for supported filter effects"),
                ),
            true,
        ))
}

/// Build the CLI argument parser and command definitions.
pub fn build_cli() -> Command {
    // Compose root args and subcommands from smaller builders to reduce structural complexity.
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
                .conflicts_with_all(["verify"]),
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
            Arg::new("max-sink-chunks")
                .long("max-sink-chunks")
                .value_name("CHUNKS")
                .default_value("0")
                .help("Maximum sink chunks queued before producer waits (0 disables)"),
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
                .help("Input .prot/.mka file, audio file, or directory of nested audio files")
                .required(false)
                .index(1),
        )
        .subcommand(build_bench_subcommand())
        .subcommand(build_verify_subcommand())
        .subcommand(build_info_subcommand())
        .subcommand(build_peaks_subcommand())
        .subcommand(build_init_subcommand())
        .subcommand(build_create_subcommand())
        .subcommand(build_meter_subcommand())
}

#[cfg(test)]
mod tests {
    use super::build_cli;

    #[test]
    fn parses_basic_input_and_defaults() {
        let matches = build_cli()
            .try_get_matches_from(["prot", "song.wav"])
            .expect("cli should parse");
        assert_eq!(
            matches.get_one::<String>("INPUT").map(String::as_str),
            Some("song.wav")
        );
        assert_eq!(
            matches.get_one::<String>("GAIN").map(String::as_str),
            Some("70")
        );
    }

    #[test]
    fn rejects_seek_with_verify_flag() {
        let result =
            build_cli().try_get_matches_from(["prot", "--seek", "12", "--verify", "song.wav"]);
        assert!(result.is_err());
    }
}
