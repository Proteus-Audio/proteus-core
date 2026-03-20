//! CLI module entry point.

pub mod args;
pub mod bench;
pub mod controls;
mod create_cmd;
mod info_cmd;
mod meter_cmd;
mod peaks_cmd;
mod playback_runner;
pub mod runner;
mod spectral_graph;
pub mod ui;
pub mod verify;

#[cfg(test)]
mod tests {
    use super::args::build_cli;

    #[test]
    fn cli_root_registers_expected_subcommands() {
        let command = build_cli();
        for name in [
            "bench", "verify", "info", "peaks", "init", "create", "meter",
        ] {
            assert!(
                command.get_subcommands().any(|sub| sub.get_name() == name),
                "missing subcommand: {name}"
            );
        }
    }
}
