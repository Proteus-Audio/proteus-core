# CLI Onboarding: Subcommands and Project Files

- [Back to CLI Onboarding](./index.md)

## Where Commands Are Defined

Clap definitions live in [`proteus-cli/src/cli/args.rs`](../../../proteus-cli/src/cli/args.rs).

This file is the source of truth for:

- top-level flags (`--gain`, buffering knobs, effect logging)
- subcommands (`bench`, `verify`, `info`, `peaks`, `create`, `init`)
- nested subcommands and argument defaults

## Where Commands Are Executed

Subcommand dispatch lives in [`proteus-cli/src/runner.rs`](../../../proteus-cli/src/runner.rs).

`runner::run(...)` matches subcommands and delegates to helpers or submodules:

- `bench` -> `cli::bench`
- `verify` -> `cli::verify`
- `info` -> local `run_info(...)`
- `peaks` -> local peaks helpers
- `create effects-json` -> emits a default effects JSON template
- `init` -> writes project files for directory playback mode

## Directory Playback Mode (`project_files.rs`)

[`proteus-cli/src/project_files.rs`](../../../proteus-cli/src/project_files.rs) supports "play a folder as a project".

It can:

- discover audio files recursively and group them
- read/write `shuffle_schedule.json`
- read `effects_chain.json`
- generate default disabled effect chains (`AudioEffect` JSON)

### Key Types

- `DirectoryPlaybackConfig`
- `JsonPathsTrack`
- `ShuffleScheduleFile`

### Why This Matters

This module is where CLI UX and library scheduling meet. It converts directory/project JSON into `Vec<PathsTrack>` for `proteus-lib`.

## `create` and `init` Workflows

- `create effects-json`: useful for bootstrapping a standalone effect chain JSON
- `init <dir>`: writes both `shuffle_schedule.json` and `effects_chain.json` into a directory project

These are productivity features for people authoring local playback projects without container files.

## Common Pitfalls

- Changing `AudioEffect` serde names in `proteus-lib` can break CLI JSON compatibility.
- Changing `PathsTrack` semantics in `proteus-lib` can break directory mode if `project_files.rs` is not updated.
- Feature additions to effect defaults should usually be reflected in `default_effects_chain_enabled/disabled`.

## Related

- [CLI Architecture and Control Flow](./architecture-and-control-flow.md)
- [Library Container and Scheduling Model](../library/container-and-scheduling.md)
- [DSP Effects and Signal Chain](../library/dsp-effects-and-signal-chain.md)
