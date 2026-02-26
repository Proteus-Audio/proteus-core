# Library Onboarding: Architecture Overview

- [Back to Library Onboarding](./index.md)

## Purpose of `proteus-lib`

`proteus-lib` is the reusable engine behind Proteus playback. It owns:

- Container/file input modeling (`Prot`)
- Real-time decode + mix + DSP playback
- Audio effect implementations
- Diagnostics/metrics surfaces used by the CLI (and potentially GUIs)

Start at [`proteus-lib/src/lib.rs`](../../../proteus-lib/src/lib.rs) for the public module map.

## Top-Level Modules (What They Do)

- [`proteus-lib/src/container/`](../../../proteus-lib/src/container/)
  - Parses `.prot` / `.mka` metadata and `play_settings.json`
  - Builds active track selections and shuffle schedules
- [`proteus-lib/src/playback/`](../../../proteus-lib/src/playback/)
  - High-level `Player` API
  - Playback thread bootstrap/worker loop
  - Mix engine and decode workers
- [`proteus-lib/src/dsp/`](../../../proteus-lib/src/dsp/)
  - Audio effect implementations and DSP utilities
- [`proteus-lib/src/track/`](../../../proteus-lib/src/track/)
  - Track decoding and buffering helpers used by playback
- [`proteus-lib/src/audio/`](../../../proteus-lib/src/audio/)
  - Shared sample/buffer primitives
- [`proteus-lib/src/diagnostics/`](../../../proteus-lib/src/diagnostics/)
  - Metrics/reporting support
- [`proteus-lib/src/peaks/`](../../../proteus-lib/src/peaks/)
  - Waveform peak extraction/read/write used by CLI subcommands

## Core Runtime Abstractions

### `Prot` (what should play)

Defined in [`proteus-lib/src/container/prot.rs`](../../../proteus-lib/src/container/prot.rs).

`Prot` is the playback model for a container or a set of standalone files. It stores:

- audio metadata (`Info`)
- selected tracks / paths
- shuffle schedule
- parsed effects from play settings (when present)
- total duration

Think of `Prot` as the "playback plan" plus metadata, not the playback engine itself.

### `Player` (public control surface)

Defined in [`proteus-lib/src/playback/player/mod.rs`](../../../proteus-lib/src/playback/player/mod.rs).

`Player` is the main integration point for callers. It owns:

- transport state (`play`, `pause`, `seek`, `stop`)
- playback thread lifecycle
- shared state (time, duration, sink, effects, metrics)
- runtime tuning knobs (buffer settings, fades, logging)

### `PlayerEngine` (mix/decode runtime engine)

Defined in [`proteus-lib/src/playback/engine/mod.rs`](../../../proteus-lib/src/playback/engine/mod.rs).

`PlayerEngine` bridges `Prot` + buffers + effect chain into a stream of mixed audio chunks sent to the playback worker.

## Mental Model: Who Owns What

- `Prot`: selection/schedule metadata and track plan
- `Player`: API + thread orchestration + shared mutable state
- `PlayerEngine`: decode/mix/effect production for one playback run
- Playback worker (`run_playback_thread`): sink append, playback timing, drain/end-of-stream handling

## Good First Source Files to Read

1. [`proteus-lib/src/playback/player/mod.rs`](../../../proteus-lib/src/playback/player/mod.rs)
2. [`proteus-lib/src/playback/player/controls.rs`](../../../proteus-lib/src/playback/player/controls.rs)
3. [`proteus-lib/src/playback/player/runtime/thread.rs`](../../../proteus-lib/src/playback/player/runtime/thread.rs)
4. [`proteus-lib/src/playback/player/runtime/worker/runner.rs`](../../../proteus-lib/src/playback/player/runtime/worker/runner.rs)
5. [`proteus-lib/src/playback/engine/mod.rs`](../../../proteus-lib/src/playback/engine/mod.rs)
6. [`proteus-lib/src/container/prot.rs`](../../../proteus-lib/src/container/prot.rs)
7. [`proteus-lib/src/dsp/effects/mod.rs`](../../../proteus-lib/src/dsp/effects/mod.rs)

## Related Knowledge Base

- [Player Data Flows](../../player/data-flows.md)
- [Run Playback Thread Sample Flow](../../player/run-playback-thread-sample-flow.md)
- [Shuffle Points in Playback](../../player/shuffle-points-playback.md)
