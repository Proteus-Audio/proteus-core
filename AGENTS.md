# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains the Rust library and CLI entrypoint.
- `src/main.rs` defines the CLI and playback workflow.
- `src/lib.rs` exposes the library surface for `proteus_audio`.
- `src/effects/` holds audio effect implementations (reverb, convolution, spring impulse response).
- `Cargo.toml` and `Cargo.lock` define crate metadata and dependencies.

## Build, Test, and Development Commands
- `cargo build` builds the library and CLI in debug mode.
- `cargo run -- <path-to-file.prot>` runs the CLI against a `.prot` or `.mka` file.
- `cargo check` performs fast type-checking without producing binaries.
- `cargo test` runs tests (none are currently defined, so this is a no-op).

## Coding Style & Naming Conventions
- Follow Rust 2021 conventions: `snake_case` for functions/modules, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Keep modules focused; audio DSP helpers generally belong under `src/effects/`.
- Prefer small, composable functions for decoding and playback pipelines.
- Use `rustfmt` (`cargo fmt`) for consistent formatting.

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
- Local test data is referenced from `src/test_data.rs`; keep any new fixtures small and documented.
