# Library Onboarding: Playback Runtime and Threading

- [Back to Library Onboarding](./index.md)

## High-Level Runtime Shape

There are two important runtime layers:

- `Player` thread orchestration and transport state
- Playback worker + `PlayerEngine` chunk production

The core files are:

- [`proteus-lib/src/playback/player/controls.rs`](../../../proteus-lib/src/playback/player/controls.rs)
- [`proteus-lib/src/playback/player/runtime/thread.rs`](../../../proteus-lib/src/playback/player/runtime/thread.rs)
- [`proteus-lib/src/playback/player/runtime/worker/runner.rs`](../../../proteus-lib/src/playback/player/runtime/worker/runner.rs)
- [`proteus-lib/src/playback/engine/mod.rs`](../../../proteus-lib/src/playback/engine/mod.rs)

## `Player` Transport Flow (What Callers Touch)

`Player` methods like `play`, `pause`, `seek`, and `stop` mostly mutate shared state and coordinate runtime rebuilds.

### `play()`

`play()` in [`proteus-lib/src/playback/player/controls.rs`](../../../proteus-lib/src/playback/player/controls.rs):

- ensures a playback thread exists (spawns one if not)
- requests `Resuming` state
- waits for audio-heard signal (bounded wait)

### `seek(ts)`

`seek()` is more disruptive:

- updates target time
- may fade out the current sink
- requests effect reset / clears inline updates
- kills current playback runtime
- spawns a fresh runtime starting at `ts`
- resumes if playback was active

This "rebuild on seek" design is simpler and safer than trying to mutate deep decode/mix state in place.

## Playback Thread Bootstrap (`initialize_thread`)

`initialize_thread(...)` in [`proteus-lib/src/playback/player/runtime/thread.rs`](../../../proteus-lib/src/playback/player/runtime/thread.rs):

1. Joins any previous playback thread
2. Resets per-run/shared counters and flags
3. Opens or reuses the output stream
4. Builds a `ThreadContext` snapshot of shared state handles
5. Spawns `run_playback_thread(...)`

Important design detail:

- The output stream is owned by `Player` and reused across runs when possible.
- The sink is recreated per playback run.

## Worker Loop (`run_playback_thread`)

`run_playback_thread(...)` in [`proteus-lib/src/playback/player/runtime/worker/runner.rs`](../../../proteus-lib/src/playback/player/runtime/worker/runner.rs) is the worker-side coordinator.

It:

1. Builds `PlayerEngine`
2. Initializes a paused/muted sink
3. Sets duration/time shared state
4. Appends optional startup silence
5. Starts engine receiver for `(SamplesBuffer, duration)` chunks
6. Appends chunks to the sink while enforcing buffering and runtime state gates
7. Runs a drain loop for effect tails
8. Applies end-of-stream action (`Stop` or `Pause`)

## `PlayerEngine` Responsibilities

`PlayerEngine` in [`proteus-lib/src/playback/engine/mod.rs`](../../../proteus-lib/src/playback/engine/mod.rs):

- prepares track ring buffers and per-track gains/pan
- spawns the mix thread (`spawn_mix_thread`)
- exposes a receiver of already-mixed chunks
- tracks finished-buffering state

Think of it as "chunk producer", while the worker loop is "device/sink coordinator".

## Why This Split Matters

This separation reduces complexity:

- mix/decode timing and device append timing are not tangled
- worker can manage sink state transitions independently
- `Player` can rebuild the entire runtime for seek/shuffle/disruptive changes

## Common Places New Bugs Hide

- State transitions (`Resuming`/`Pausing`/`Stopping`) vs actual sink state
- Shared atomics/flags reset order during `initialize_thread`
- Drain behavior with tail-producing effects (reverb/convolution)
- Seek behavior when inline effect updates are pending

## Related Knowledge Base

- [Player Data Flows](../../player/data-flows.md)
- [Run Playback Thread Sample Flow](../../player/run-playback-thread-sample-flow.md)
- [Set Effects Inline](../../player/set-effects-inline.md)
- [Convolution Reverb Boundary Discontinuity](../../convolution-reverb/boundary-discontinuity.md)
