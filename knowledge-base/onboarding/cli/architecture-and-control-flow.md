# CLI Onboarding: Architecture and Control Flow

- [Back to CLI Onboarding](./index.md)

## Entry Point

The CLI starts in [`proteus-cli/src/main.rs`](../../../proteus-cli/src/main.rs):

1. Build Clap args
2. Initialize logging
3. Load `.env` (optional)
4. Delegate to `runner::run(...)`
5. Exit with returned status code

This keeps `main.rs` intentionally thin.

## Main Execution Router

The real CLI control flow lives in [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs).

`run(...)` does two major things:

- dispatches explicit subcommands (`bench`, `verify`, `info`, `peaks`, `create`, `init`)
- or enters playback mode (default path when an input is provided without subcommands)

## CLI Module Roles

- [`proteus-cli/src/cli/args.rs`](../../../proteus-cli/src/cli/args.rs)
  - Centralizes Clap command/arg definitions
- [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs)
  - Main mode dispatch and playback loop orchestration
- [`proteus-cli/src/controls.rs`](../../../proteus-cli/src/controls.rs)
  - Key handling + status text formatting
- [`proteus-cli/src/ui.rs`](../../../proteus-cli/src/ui.rs)
  - Ratatui rendering (title/status/logs/meters)
- [`proteus-cli/src/project_files.rs`](../../../proteus-cli/src/project_files.rs)
  - Directory mode config discovery/init (`shuffle_schedule.json`, `effects_chain.json`)
- [`proteus-cli/src/logging.rs`](../../../proteus-cli/src/logging.rs)
  - Log buffering and stderr capture for TUI display

## Input Modes the CLI Supports

The default playback path can accept:

- `.prot` / `.mka` container
- a directory of audio files (with optional project config files)
- a single audio file path (legacy/simple mode)

The runner decides which `Player` constructor to use based on path type.

## Why This Structure Is Helpful

- Clap definitions stay readable and testable in one file
- UI rendering is decoupled from input handling
- `runner.rs` owns orchestration, but not every implementation detail
- `proteus-lib` stays the source of playback truth

## Related

- [CLI Playback Mode and TUI Loop](./playback-mode-and-tui.md)
- [CLI Subcommands and Project Files](./subcommands-and-project-files.md)
- [Library Architecture Overview](../library/architecture-overview.md)
