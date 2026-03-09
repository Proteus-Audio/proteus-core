# proteus-core

Rust workspace for Proteus container playback and tooling.

## Workspace

- `proteus-lib`: core library for container parsing, runtime planning, decode/mix orchestration, DSP effects, and peaks extraction.
- `proteus-cli`: command-line playback app and operational commands (`info`, `peaks`, `verify`, `create`, `init`).
- `proteus-scripts`: maintenance and utility scripts.

## Runtime Architecture

1. CLI builds `Player` with source input (`.prot`, `.mka`, file paths, or directory project files).
2. `container` modules parse metadata, play settings, and runtime source plans.
3. `playback::engine::mix` spawns decode workers and routes decoded packets through `BufferMixer`.
4. `BufferMixer` aligns source windows and emits synchronized mixed chunks.
5. DSP effects process each mixed chunk before output sink append.

Primary entrypoints:

- CLI: `proteus-cli/src/main.rs`
- Player API: `proteus-lib/src/playback/player/mod.rs`
- Mix runtime: `proteus-lib/src/playback/engine/mix/`

## Build and Development

- `cargo check`
- `cargo fmt`
- `cargo clippy -- -D warnings`
- `cargo test`

Run CLI:

- `cargo run -p proteus-cli -- <file.prot>`
- `cargo run -p proteus-cli -- info <file>`
- `cargo run -p proteus-cli -- peaks json <file>`

## Data and Compatibility Notes

- `.prot` / `.mka` playback settings are read from container payload metadata.
- Backward-compatible parsing for older play-settings/effects remains at deserialize boundaries.
- Real-time code paths should avoid blocking and unnecessary allocation in hot loops.

## Project History

Historical context and earlier architecture transitions are documented in [`docs/history.md`](docs/history.md).

## License

This project is licensed under the [PolyForm Small Business License 1.0.0](https://polyformproject.org/licenses/small-business/1.0.0/).

- Free for individuals and businesses under $1M USD annual revenue.
- Commercial license required for larger organizations.
- Contact [Adam Howard](mailto:adam.thomas.howard@gmail.com) for commercial licensing.
