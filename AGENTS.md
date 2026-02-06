# Repository Guidelines

## Project Structure & Module Organization
- Workspace root `Cargo.toml` defines members.
- `proteus-lib/` is the library crate.
- `proteus-lib/src/lib.rs` exposes the `proteus_lib` API.
- `proteus-lib/src/effects/` holds audio effect implementations (reverb, convolution, spring impulse response).
- `proteus-cli/` is the CLI crate.
- `proteus-cli/src/main.rs` defines the CLI and playback workflow.
- `Cargo.lock` captures resolved dependencies for the workspace.

## Build, Test, and Development Commands
- `cargo build` builds all workspace members in debug mode.
- `cargo run -p proteus-cli -- <path-to-file.prot>` runs the CLI against a `.prot` or `.mka` file.
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

## Persistent Notes
- See `NOTES.md` for playback alignment constraints that should always be preserved.
