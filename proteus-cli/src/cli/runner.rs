//! CLI dispatch entrypoint.

use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
};

use clap::ArgMatches;
use log::{error, info};
use symphonia::core::errors::Result;

use crate::logging::{self, LogLine, StderrCaptureGuard};
use crate::{cli, project_files};

use super::{create_cmd, info_cmd, meter_cmd, peaks_cmd, playback_runner};

/// Main CLI execution path: parse args, run subcommands, or start playback.
pub fn run(args: &ArgMatches, log_buffer: Arc<Mutex<VecDeque<LogLine>>>) -> Result<i32> {
    let startup_stderr_capture = start_tui_log_capture(args, &log_buffer);
    // The offline benchmark supplies concise, case-specific progress on stderr.
    // Avoid emitting an unrelated startup diagnostic before it can take over.
    if !matches!(args.subcommand(), Some(("bench", _))) {
        info!("Starting Proteus CLI");
    }

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
            "meter" => meter_cmd::run_meter_subcommand(sub_args)?,
            "create" => match sub_args.subcommand() {
                Some(("effects-json", _)) => create_cmd::run_create_effects_json(),
                Some(("prot", create_args)) => create_cmd::run_create_prot(create_args),
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

    playback_runner::run_playback(args, log_buffer, startup_stderr_capture)
}

/// TUI playback owns stderr from the first startup message onward, so startup
/// diagnostics arrive in the same buffer as runtime diagnostics.
fn start_tui_log_capture(
    args: &ArgMatches,
    log_buffer: &Arc<Mutex<VecDeque<LogLine>>>,
) -> Option<StderrCaptureGuard> {
    if !uses_tui(args) {
        return None;
    }

    logging::set_echo_stderr(false);
    let capture = logging::capture_stderr(log_buffer.clone());
    if capture.is_none() {
        // Do not hide diagnostics when the OS-level redirection cannot be
        // established (for example, when stderr is unavailable).
        logging::set_echo_stderr(true);
    }
    capture
}

fn uses_tui(args: &ArgMatches) -> bool {
    args.subcommand().is_none()
        && !args.get_flag("quiet")
        && !args.get_flag("scan-durations")
        && !args.get_flag("read-durations")
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
        "supported" => cli::verify::VerifyMode::Supported,
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

#[cfg(test)]
mod tests {
    fn empty_log_buffer(
    ) -> std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<crate::logging::LogLine>>> {
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()))
    }

    #[test]
    fn verify_without_nested_subcommand_returns_error_code() {
        let args = clap::Command::new("prot")
            .subcommand(clap::Command::new("verify"))
            .get_matches_from(["prot", "verify"]);

        let code = super::run(&args, empty_log_buffer()).expect("runner should return result");
        assert_eq!(code, -1);
    }

    #[test]
    fn init_with_missing_directory_returns_error_code() {
        let args = crate::cli::args::build_cli()
            .try_get_matches_from(["prot", "init", "/definitely/missing/dir"])
            .expect("cli should parse");

        let code = super::run(&args, empty_log_buffer()).expect("runner should return result");
        assert_eq!(code, -1);
    }

    #[test]
    fn tui_capture_is_limited_to_interactive_playback() {
        let interactive = crate::cli::args::build_cli()
            .try_get_matches_from(["prot", "test_audio/demo-effects.prot"])
            .expect("interactive arguments should parse");
        let quiet = crate::cli::args::build_cli()
            .try_get_matches_from(["prot", "--quiet", "test_audio/demo-effects.prot"])
            .expect("quiet arguments should parse");
        let scan = crate::cli::args::build_cli()
            .try_get_matches_from(["prot", "--scan-durations", "test_audio/demo-effects.prot"])
            .expect("scan arguments should parse");
        let subcommand = crate::cli::args::build_cli()
            .try_get_matches_from(["prot", "info", "test_audio/demo-effects.prot"])
            .expect("subcommand arguments should parse");

        assert!(super::uses_tui(&interactive));
        assert!(!super::uses_tui(&quiet));
        assert!(!super::uses_tui(&scan));
        assert!(!super::uses_tui(&subcommand));
    }
}
