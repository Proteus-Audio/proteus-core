//! # Prot Play
//!
//! Command-line player for `.prot`/`.mka` containers backed by `proteus-lib`.

use dotenv::dotenv;
use log::error;

mod cli;
mod controls;
mod logging;
mod runner;
mod ui;

/// Entry point for the CLI binary.
fn main() {
    let args = cli::args::build_cli().get_matches();
    let log_buffer = logging::init();
    dotenv().ok();

    let code = match runner::run(&args, log_buffer) {
        Ok(code) => code,
        Err(err) => {
            error!("{}", err.to_string().to_lowercase());
            -1
        }
    };

    std::process::exit(code)
}
