use std::env;
use std::fs;
use std::path::PathBuf;

use proteus_lib::dsp::impulse_response::normalize_impulse_response_channels;
use proteus_lib::dsp::spring_impulse_response::SPRING_IMPULSE_RESPONSE;

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
    let mut tail_db: Option<f32> = Some(-60.0);
    let mut name = String::from("NORMALIZED_SPRING_IMPULSE_RESPONSE");

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
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
            "--name" => {
                if let Some(value) = iter.next() {
                    name = value;
                } else {
                    eprintln!("--name requires a value");
                    return;
                }
            }
            "-h" | "--help" => {
                print_normalize_help();
                return;
            }
            _ => {
                eprintln!("Unknown normalize arg: {}", arg);
                print_normalize_help();
                return;
            }
        }
    }

    let mut channels = vec![SPRING_IMPULSE_RESPONSE.to_vec()];
    normalize_impulse_response_channels(&mut channels, tail_db);
    let normalized = &channels[0];

    let mut out = String::new();
    out.push_str(&format!("pub const {}: &[f32] = &[\n", name));
    for (idx, sample) in normalized.iter().enumerate() {
        if idx % 12 == 0 {
            out.push_str("    ");
        }
        out.push_str(&format!("{:.7},", sample));
        if idx % 12 == 11 {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("];\n");

    match out_path {
        Some(path) => {
            if let Err(err) = fs::write(&path, out) {
                eprintln!("Failed to write {}: {}", path.display(), err);
            } else {
                println!("Wrote {}", path.display());
            }
        }
        None => {
            println!("{}", out);
        }
    }
}

fn print_help() {
    println!(
        "proteus-scripts\n\nCommands:\n  normalize    Create a normalized spring impulse response\n\nRun 'proteus-scripts normalize --help' for options."
    );
}

fn print_normalize_help() {
    println!(
        "Usage: proteus-scripts normalize [options]\n\nOptions:\n  --out <path>       Write output to file instead of stdout\n  --tail-db <db>     Tail trim threshold (default -60)\n  --no-tail          Disable tail trim\n  --name <ident>     Constant name (default NORMALIZED_SPRING_IMPULSE_RESPONSE)\n  -h, --help         Show this help"
    );
}
