# Repository Guidelines

## Project Structure & Module Organization
This is a Rust workspace. `Cargo.toml` at the root defines members:
- `proteus-lib/`: core library crate (`proteus-lib/src/lib.rs`) and shared playback/container logic.
- `proteus-cli/`: CLI entrypoint and playback workflow (`proteus-cli/src/main.rs`).
- `proteus-scripts/`: helper utilities (for example impulse response normalization).

Within `proteus-lib/src/`:
- `dsp/` contains audio DSP implementations (reverb, convolution, impulse responses).
- `playback/` contains the real-time player and mixing engine.
- `container/` handles `.prot`/`.mka` parsing, settings, and metadata scanning.
- `track/` handles per-track decoding and buffering.

## Build, Test, and Development Commands
- `cargo check`: fast type-check across the workspace (run before finishing changes).
- `cargo build`: build all crates in debug mode.
- `cargo fmt`: format Rust sources.
- `cargo clippy -- -D warnings`: run lints and fail on warnings.
- `cargo test`: run tests (coverage is currently minimal).
- `cargo run -p proteus-cli -- <file.prot>`: run CLI playback on a `.prot`/`.mka` file.
- `cargo run -p proteus-cli --features debug -- <path>`: run CLI with debug playback metrics.

## Coding Style & Naming Conventions
- Follow Rust 2021 conventions: `snake_case` for functions/modules, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Use 4 spaces for indentation (rustfmt default) and avoid emoji/unicode symbols in source.
- Keep modules focused; audio DSP helpers generally belong under `src/dsp/`.
- Always keep the codebase as clean and readable as possible; add proper documentation comments and split modules into multiple files when complexity grows.
- Prefer small, composable functions for decoding, buffering, and playback pipelines.
- Use `rustfmt` (`cargo fmt`) for consistent formatting.
- Public items in `proteus-lib` should have doc comments. Include `# Arguments`, `# Returns`, and `# Errors`
  sections when the signature warrants it, and add examples for complex functions.
- Real-time paths should avoid unnecessary allocations and long critical sections.

## Testing Guidelines
No formal test suite is established yet. Add unit tests inline (`mod tests`) for library behavior and integration tests under `tests/` for CLI behavior. Use descriptive names such as `decodes_valid_prot_header`.

## Commit & Pull Request Guidelines
Use short, imperative commit messages (for example, `Refactor playback status reporting`). PRs should describe behavior changes, include validation steps (for example `cargo check`, `cargo run ...`), and note any sample inputs/flags used.

## Configuration & Data Notes
- The CLI expects standard audio formats, `.prot`, or `.mka` inputs and will error otherwise.
- Local test data is referenced from `proteus-lib/src/test_data.rs`; keep any new fixtures small and documented.
- Container playback settings are read from `play_settings.json` inside `.prot`/`.mka` files.
- Feature flags: `bench`, `debug`, and `real-fft` are supported in both `proteus-lib` and `proteus-cli`.
- Logging uses the `log` crate (not `tracing`) across the workspace.
- The library currently contains `.unwrap()`/`.expect()` calls; avoid adding new ones in library code unless an invariant is guaranteed.
- If introducing new error types, prefer `thiserror` in `proteus-lib` and `anyhow` in `proteus-cli`.

## Persistent Notes
- See `.guides/NOTES.md` for playback alignment constraints that should always be preserved.
- In this repository, "knowledge base" always refers to the `knowledge-base/` directory.
- Keep `knowledge-base/` documents up to date when `proteus-lib` behavior, algorithms, or effect implementations change.
