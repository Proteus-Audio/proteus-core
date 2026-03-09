//! CLI dispatch entrypoint.

use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
};

use clap::ArgMatches;
use log::{error, info};
use symphonia::core::errors::Result;

use crate::logging::LogLine;
use crate::{cli, project_files};

use super::{create_cmd, info_cmd, peaks_cmd, playback_runner};

/// Main CLI execution path: parse args, run subcommands, or start playback.
pub fn run(args: &ArgMatches, log_buffer: Arc<Mutex<VecDeque<LogLine>>>) -> Result<i32> {
    info!("Starting Proteus CLI");

    if let Some((subcommand, sub_args)) = args.subcommand() {
        return Ok(match subcommand {
            "bench" => cli::bench::run_bench_subcommand(sub_args)?,
            "info" => {
                let file_path = sub_args.get_one::<String>("INPUT").unwrap();
                let print = sub_args.get_flag("print");
                info_cmd::run_info(file_path, print)
            }
            "peaks" => peaks_cmd::run_peaks(sub_args),
            "verify" => run_verify(sub_args)?,
            "create" => match sub_args.subcommand() {
                Some(("effects-json", _)) => create_cmd::run_create_effects_json(),
                _ => {
                    error!("Unknown create subcommand");
                    -1
                }
            },
            "init" => run_init(sub_args),
            _ => {
                error!("Unknown subcommand");
                -1
            }
        });
    }

    playback_runner::run_playback(args, log_buffer)
}

fn run_verify(args: &ArgMatches) -> Result<i32> {
    let (verify_cmd, verify_args) = match args.subcommand() {
        Some((cmd, args)) => (cmd, args),
        None => {
            error!("Missing verify subcommand");
            return Ok(-1);
        }
    };
    let file_path = verify_args.get_one::<String>("INPUT").unwrap();
    let mode = match verify_cmd {
        "probe" => cli::verify::VerifyMode::Probe,
        "decode" => cli::verify::VerifyMode::Decode,
        "verify" => cli::verify::VerifyMode::Verify,
        _ => {
            error!("Unknown verify subcommand");
            return Ok(-1);
        }
    };
    cli::verify::run_verify(file_path, mode)
}

fn run_init(args: &ArgMatches) -> i32 {
    let dir = args.get_one::<String>("INPUT").unwrap();
    match project_files::write_init_files(Path::new(dir)) {
        Ok(()) => 0,
        Err(err) => {
            error!("{}", err);
            -1
        }
    }
}
