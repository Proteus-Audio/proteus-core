//! # Prot Play
//!
//! A command-line audio player for the Prot audio format.

use log::error;

mod cli;
mod controls;
mod runner;
mod ui;

fn main() {
    let args = cli::args::build_cli().get_matches();

    let code = match runner::run(&args) {
        Ok(code) => code,
        Err(err) => {
            error!("{}", err.to_string().to_lowercase());
            -1
        }
    };

    std::process::exit(code)
}
