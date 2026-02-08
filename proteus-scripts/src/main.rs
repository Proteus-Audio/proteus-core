use std::env;
use std::path::PathBuf;

use hound::{SampleFormat, WavSpec, WavWriter};
use proteus_lib::dsp::effects::convolution_reverb::impulse_response::{
    load_impulse_response_from_file_with_tail, normalize_impulse_response_channels,
};

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        print_help();
        return;
    };

    match cmd.as_str() {
        "normalize" => normalize_cmd(args.collect()),
        "-h" | "--help" => print_help(),
        _ => {
            eprintln!("Unknown command: {}", cmd);
            print_help();
        }
    }
}

fn normalize_cmd(args: Vec<String>) {
    let mut out_path: Option<PathBuf> = None;
    let mut in_path: Option<PathBuf> = None;
    let mut tail_db: Option<f32> = Some(-60.0);

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--in" => {
                if let Some(path) = iter.next() {
                    in_path = Some(PathBuf::from(path));
                } else {
                    eprintln!("--in requires a path");
                    return;
                }
            }
            "--out" => {
                if let Some(path) = iter.next() {
                    out_path = Some(PathBuf::from(path));
                } else {
                    eprintln!("--out requires a path");
                    return;
                }
            }
            "--tail-db" => {
                if let Some(value) = iter.next() {
                    match value.parse::<f32>() {
                        Ok(val) => tail_db = Some(val),
                        Err(_) => {
                            eprintln!("Invalid --tail-db value: {}", value);
                            return;
                        }
                    }
                } else {
                    eprintln!("--tail-db requires a value");
                    return;
                }
            }
            "--no-tail" => {
                tail_db = None;
            }
            "-h" | "--help" => {
                print_normalize_help();
                return;
            }
            value if !value.starts_with("--") => {
                if in_path.is_none() {
                    in_path = Some(PathBuf::from(value));
                } else if out_path.is_none() {
                    out_path = Some(PathBuf::from(value));
                } else {
                    eprintln!("Unexpected extra argument: {}", value);
                    print_normalize_help();
                    return;
                }
            }
            _ => {
                eprintln!("Unknown normalize arg: {}", arg);
                print_normalize_help();
                return;
            }
        }
    }

    let Some(in_path) = in_path else {
        eprintln!("Missing input file path");
        print_normalize_help();
        return;
    };
    let Some(out_path) = out_path else {
        eprintln!("Missing output file path");
        print_normalize_help();
        return;
    };

    let impulse_response = match load_impulse_response_from_file_with_tail(&in_path, tail_db) {
        Ok(ir) => ir,
        Err(err) => {
            eprintln!("Failed to load impulse response: {}", err);
            return;
        }
    };

    let mut channels = impulse_response.channels;
    normalize_impulse_response_channels(&mut channels, tail_db);

    if let Err(err) = write_wav(&out_path, impulse_response.sample_rate, &channels) {
        eprintln!("Failed to write {}: {}", out_path.display(), err);
        return;
    }

    println!("Wrote {}", out_path.display());
}

fn print_help() {
    println!(
        "proteus-scripts\n\nCommands:\n  normalize    Normalize an impulse response audio file\n\nRun 'proteus-scripts normalize --help' for options."
    );
}

fn print_normalize_help() {
    println!(
        "Usage: proteus-scripts normalize <input> <output> [options]\n\nOptions:\n  --in <path>        Input audio file path\n  --out <path>       Output wav path\n  --tail-db <db>     Tail trim threshold (default -60)\n  --no-tail          Disable tail trim\n  -h, --help         Show this help"
    );
}

fn write_wav(path: &PathBuf, sample_rate: u32, channels: &[Vec<f32>]) -> Result<(), String> {
    let channel_count = channels.len().max(1) as u16;
    let max_len = channels.iter().map(|ch| ch.len()).max().unwrap_or(0);
    let spec = WavSpec {
        channels: channel_count,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    let mut writer = WavWriter::create(path, spec)
        .map_err(|err| format!("failed to create wav writer: {}", err))?;

    for frame in 0..max_len {
        for ch in 0..channel_count as usize {
            let sample = channels
                .get(ch)
                .and_then(|data| data.get(frame))
                .copied()
                .unwrap_or(0.0);
            writer
                .write_sample(sample)
                .map_err(|err| format!("failed to write sample: {}", err))?;
        }
    }

    writer
        .finalize()
        .map_err(|err| format!("failed to finalize wav: {}", err))?;

    Ok(())
}
