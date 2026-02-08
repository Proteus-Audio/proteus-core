# Repository Guidelines

## Project Structure & Module Organization
- Workspace root `Cargo.toml` defines members.
- `proteus-lib/` is the library crate.
- `proteus-lib/src/lib.rs` exposes the `proteus_lib` API surface.
- `proteus-lib/src/dsp/` holds audio DSP implementations (convolution, reverb, impulse responses).
- `proteus-lib/src/playback/` contains the real-time player and mixing engine.
- `proteus-lib/src/container/` owns `.prot`/`.mka` parsing, play settings, and metadata scans.
- `proteus-lib/src/track/` owns per-track decoding and buffering.
- `proteus-cli/` is the CLI crate.
- `proteus-cli/src/main.rs` defines the CLI and playback workflow.
- `proteus-scripts/` is a small helper CLI (currently for impulse response normalization).
- `play_settings_guide.rs` is a local reference for play settings schema details.
- `Cargo.lock` captures resolved dependencies for the workspace.

## Build, Test, and Development Commands
- `cargo build` builds all workspace members in debug mode.
- `cargo run -p proteus-cli -- <path-to-file.prot>` runs the CLI against a `.prot` or `.mka` file.
- `cargo run -p proteus-cli --features debug -- <path>` enables extra playback metrics in the TUI.
- `cargo run -p proteus-scripts -- normalize --help` shows helper script usage.
- `cargo check` performs fast type-checking without producing binaries.
- `cargo fmt` formats Rust sources.
- `cargo clippy -- -D warnings` runs the lints used by this codebase.
- `cargo test` runs tests (none are currently defined today).
- `cargo check` should pass before finishing any task.

## Coding Style & Naming Conventions
- Follow Rust 2021 conventions: `snake_case` for functions/modules, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Use 4 spaces for indentation (rustfmt default) and avoid emoji/unicode symbols in source.
- Keep modules focused; audio DSP helpers generally belong under `src/dsp/`.
- Prefer small, composable functions for decoding, buffering, and playback pipelines.
- Use `rustfmt` (`cargo fmt`) for consistent formatting.
- Public items in `proteus-lib` should have doc comments. Include `# Arguments`, `# Returns`, and `# Errors`
  sections when the signature warrants it, and add examples for complex functions.
- Real-time paths should avoid unnecessary allocations and long critical sections.

## Testing Guidelines
- No unit or integration tests are present today.
- When adding tests, prefer:
  - Unit tests in the module file (`mod tests { ... }`).
  - Integration tests under `tests/` for CLI behavior.
- Name test functions descriptively, e.g., `decodes_valid_prot_header`.

## Commit & Pull Request Guidelines
- Commit history uses short, imperative summaries (e.g., `Update Cargo.toml`, `Refactor reporting status`). Keep messages concise and action-focused.
- PRs should include:
  - A clear description of behavior changes.
  - Steps to reproduce or validate (`cargo run -- ...` or `cargo test`).
  - Any relevant sample input files or flags used.

## Configuration & Data Notes
- The CLI expects `.prot` or `.mka` inputs and will error otherwise.
- Local test data is referenced from `proteus-lib/src/test_data.rs`; keep any new fixtures small and documented.
- Container playback settings are read from `play_settings.json` inside `.prot`/`.mka` files.
- Feature flags: `bench`, `debug`, and `real-fft` are supported in both `proteus-lib` and `proteus-cli`.
- Logging uses the `log` crate (not `tracing`) across the workspace.
- The library currently contains `.unwrap()`/`.expect()` calls; avoid adding new ones in library code unless an invariant is guaranteed.
- If introducing new error types, prefer `thiserror` in `proteus-lib` and `anyhow` in `proteus-cli`.

## Persistent Notes
- See `.guides/NOTES.md` for playback alignment constraints that should always be preserved.
