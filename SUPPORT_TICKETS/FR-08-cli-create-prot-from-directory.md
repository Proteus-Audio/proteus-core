# FR-08: CLI `.prot` Creation From Directory Projects

## Summary

Add CLI support for creating `.prot` files from a folder structure organized the
same way as current directory playback. The CLI implementation should call the
feature-gated muxing API from `FR-07` and display an `indicatif` progress bar
while the file is being created.

The intended input is an existing directory project:

- nested audio folders/files
- optional `shuffle_schedule.json`
- optional `effects_chain.json`

The output is a portable `.prot` file that the existing `prot` playback path can
open.

---

## Motivation

The CLI already supports direct playback from a directory of nested audio files.
That is useful while authoring, but it does not produce a portable single file.

Users should be able to take the same folder they can play today and run one
command to package it into a `.prot` container, preserving shuffle grouping,
track levels/pan, selection counts, shuffle points, and effects.

---

## Current Gap

### A. Directory playback has the right discovery semantics

[`proteus-cli/src/project_files.rs`](../proteus-cli/src/project_files.rs)
currently implements:

- `load_directory_playback_config(root)`
- `discover_audio_groups(root)`
- `shuffle_schedule.json` parsing
- `effects_chain.json` loading
- fallback discovery when `shuffle_schedule.json` is absent

The create command should reuse these semantics so "folder playback" and
"folder muxing" agree about what a project means.

### B. The current `create` command only emits JSON

[`proteus-cli/src/cli/args.rs`](../proteus-cli/src/cli/args.rs) defines
`create effects-json`, and
[`proteus-cli/src/cli/create_cmd.rs`](../proteus-cli/src/cli/create_cmd.rs)
prints default effect settings.

There is no `create prot` / `create package` command that consumes a directory
and writes a `.prot` file.

### C. The mux demo includes progress UX

`~/Dev/proteus/mux-demo/src/mux.rs` uses `indicatif` with phases such as:

- transcoding input files
- ordering packets
- preparing output
- embedding play settings
- muxing packets
- finalizing output

The production CLI should use the same general feedback pattern, wired through
the progress interface provided by `FR-07`.

---

## Desired Outcome

Add a CLI command that creates a `.prot` file from a directory project.

Recommended command shape:

```text
prot create prot INPUT_DIR OUTPUT.prot
```

Potential flags:

```text
--title TITLE
--force
--attach PATH
--no-effects
```

Exact naming can be adjusted to match CLI conventions, but the command must be
discoverable under the existing `create` namespace and should not collide with
default playback.

---

## Proposed Design

### A. Reuse directory project loading

The command should start from `project_files::load_directory_playback_config`.

Behavior:

- if `shuffle_schedule.json` exists, use it as the source of track groups and
  per-track playback settings
- if absent, discover audio files using the same recursive folder grouping used
  for directory playback
- if `effects_chain.json` exists, embed those effects in `play_settings.json`
- if absent, embed an empty effect chain or the same disabled-default semantics
  chosen by the current project-file helpers

### B. Serialize play settings for the muxed output

The CLI should build a versioned `play_settings.json` payload using existing
library play-settings models rather than string concatenation.

Required mapping:

- each muxed audio source receives a deterministic track number
- each folder playback `PathsTrack` maps its selected file IDs to the muxed
  track numbers
- `level`, `pan`, `selections_count`, and `shuffle_points` are preserved
- names/safe names should remain useful for display after the source directory
  is no longer present

This is the subtle part of the ticket: folder playback groups can contain
multiple candidate file paths, while the container needs stable numeric track
IDs and play settings that reference those IDs.

### C. Call the library muxer

The CLI should depend on `proteus-lib` with the new `prot-mux` feature when the
CLI feature for creation is enabled.

Recommended feature wiring:

```toml
prot-mux = ["proteus-lib/prot-mux", "dep:indicatif"]
```

`indicatif` should be a CLI dependency, not a core-library API type.

### D. Display progress with `indicatif`

The command should show a progress bar while creating the file.

Recommended display phases:

- scanning project
- transcoding audio
- embedding play settings
- embedding attachments
- writing packets
- finalizing output

Implementation should gracefully disable or simplify progress in non-interactive
contexts if needed, but ordinary terminal use should show progress by default.

### E. Validate output by reading it back

After mux completion, the command should perform a cheap sanity check by opening
the output through the existing container read path. This catches mismatched
track IDs, missing `play_settings.json`, and invalid mux output before reporting
success.

---

## Acceptance Criteria

- A documented CLI command can create `OUTPUT.prot` from `INPUT_DIR`.
- The command uses the same directory project semantics as current folder
  playback.
- The command embeds `play_settings.json` with preserved track settings and
  effects.
- The command uses an `indicatif` progress bar during creation.
- The command refuses to overwrite an existing output path unless an explicit
  overwrite flag is provided.
- The output `.prot` can be played by the existing CLI/player path.
- Tests cover at least:
  - discovered directory without `shuffle_schedule.json`
  - directory with `shuffle_schedule.json`
  - directory with `effects_chain.json`
  - refusal to overwrite without `--force`

---

## Dependency

This ticket depends on `FR-07` exposing a feature-gated library muxing API.

