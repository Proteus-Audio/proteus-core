# FR-07: Feature-Gated `.prot` Muxing API

## Summary

Add opt-in muxing support to `proteus-lib` so downstream users can create
Proteus `.prot` container files from Rust code, using the published
`proteus-muxer` crate.

This should be behind a new library feature flag so the default playback-only
library does not pull muxing, encoding, or progress-oriented dependencies into
normal consumers. The first implementation should follow the working shape from
`~/Dev/proteus/mux-demo`: transcode supported source audio to Vorbis, build
Matroska audio tracks through the muxer, embed `play_settings.json`, write
packets in timestamp order, and finalize a `.prot`/`.mka`-compatible file.

The demo currently imports the muxer as `proteus_mux` while the published Cargo
package is expected to be `proteus-muxer`; implementation should confirm the
package/lib-name pairing and document it in `proteus-lib/Cargo.toml`.

---

## Motivation

`proteus-lib` can currently read and play `.prot` containers, and the CLI can
play a folder organized as a directory-backed Proteus project. There is no
library API for producing a `.prot` file from that same kind of source data.

That leaves `.prot` creation outside the core crate even though the muxing
format now has a dedicated crate. Adding a small feature-gated creation API
would make `.prot` files first-class in both directions:

- applications can package folder-backed Proteus projects into portable
  containers
- the CLI can call library code instead of carrying separate muxing logic
- muxing dependencies stay absent unless a caller explicitly opts in

---

## Current Gap

### A. Container read/play support exists, but write support does not

[`proteus-lib/src/container/prot`](../proteus-lib/src/container/prot) parses
container metadata, tracks, attachments, and `play_settings.json`.

There is no parallel API under `proteus-lib` for creating a container, choosing
track IDs, attaching play settings, or finalizing output.

### B. Directory playback data is CLI-only

[`proteus-cli/src/project_files.rs`](../proteus-cli/src/project_files.rs)
already knows how to load:

- discovered nested audio files
- `shuffle_schedule.json`
- `effects_chain.json`

But that logic produces `PathsTrack` values for playback, not a reusable
library-level muxing input model.

### C. The mux demo proves the core mechanics

`~/Dev/proteus/mux-demo/src/mux.rs` demonstrates the intended flow:

- transcode each input file with Symphonia + Vorbis
- create one muxer track per audio source with `VorbisTrackConfig`
- attach `play_settings.json`
- sort packets by `(timestamp, track_number)`
- call `MkaMuxer::builder(output)` and `write_track_packet(...)`
- call `finish()`

The library implementation should adapt that flow into a reusable API rather
than copying demo-only CLI assumptions.

---

## Desired Outcome

With a new feature flag enabled, `proteus-lib` should expose a stable, typed API
that can create a `.prot` file from source audio paths and a Proteus play
settings payload.

Recommended feature name:

```toml
prot-mux = ["dep:proteus-muxer", "dep:vorbis_rs", ...]
```

The exact dependency list should be decided during implementation, but normal
`proteus-lib` builds without `prot-mux` should remain playback-only.

The public API should support at least:

- output path or writer selection
- source audio path list / grouped track input
- optional container title / writing app metadata
- `play_settings.json` attachment bytes or typed settings serialization
- optional extra attachments
- deterministic track IDs
- progress reporting hooks without depending on `indicatif` in the library API

---

## Proposed Design

### A. Add a muxing module behind a feature flag

Recommended module shape:

```rust
#[cfg(feature = "prot-mux")]
pub mod mux;
```

Possible public types:

```rust
pub struct ProtMuxOptions {
    pub title: Option<String>,
    pub writing_app: Option<String>,
}

pub struct ProtMuxInput {
    pub output: PathBuf,
    pub tracks: Vec<ProtMuxTrackInput>,
    pub play_settings_json: Vec<u8>,
    pub attachments: Vec<ProtMuxAttachment>,
}

pub struct ProtMuxTrackInput {
    pub source_path: PathBuf,
    pub display_name: Option<String>,
}

pub trait ProtMuxProgress {
    fn set_phase(&mut self, phase: ProtMuxPhase);
    fn increment(&mut self, units: u64);
    fn add_work(&mut self, units: u64);
}
```

This is illustrative, not prescriptive. The important constraint is that the
library exposes domain-level muxing state, not an `indicatif` type.

### B. Keep encoding and packet ordering internal

The demo currently uses:

- Symphonia decoding
- `vorbis_rs::VorbisEncoderBuilder`
- Ogg packet extraction/timing helpers
- `proteus_mux::{MkaMuxer, VorbisTrackConfig, VorbisHeaders}`

The library should hide those details behind `create_prot(...)` or equivalent.
Callers should not need to understand Ogg Vorbis header packets to create a
valid Proteus container.

### C. Reuse existing play-settings models

The muxing API should use existing `PlaySettingsFile` / `SettingsTrack` models
where practical rather than hand-building JSON strings like the demo does.

Required behavior:

- embed the attachment with the exact name `play_settings.json`
- preserve supported versioned play-settings format
- ensure track IDs in play settings match muxer track numbers
- preserve `level`, `pan`, `selections_count`, and `shuffle_points`

### D. Provide progress without coupling to CLI rendering

The CLI requirement needs an `indicatif` progress bar, but `proteus-lib` should
not force `indicatif` on all muxing consumers.

Recommended approach:

- expose a callback, trait, or event iterator from the muxing function
- report coarse phases such as decode/transcode, packet ordering, attachment
  embedding, mux writing, and finalization
- include enough total-work information for the CLI to set/update a progress bar

---

## Acceptance Criteria

- `proteus-lib` has a new opt-in feature for `.prot` creation.
- Builds without the feature do not compile or expose muxing APIs and do not
  pull muxing/encoding-only dependencies.
- Builds with the feature can create a `.prot` file containing multiple audio
  tracks and a `play_settings.json` attachment.
- Created files are playable by the existing `Player::new(...)` container path.
- Track IDs in muxed audio and play settings are deterministic and aligned.
- Errors are surfaced as typed library errors with useful source-path context.
- The library progress interface is independent of `indicatif`.
- Unit or integration tests cover at least a two-track mux with play settings
  and a read/playback metadata sanity check.

---

## Out Of Scope

- Designing a new `.prot` container format.
- Adding a GUI authoring workflow.
- Making `indicatif` part of the core library API.
- Supporting every possible Symphonia codec in the first pass beyond the
  project’s already supported source-audio expectations.

