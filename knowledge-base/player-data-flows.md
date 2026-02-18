# Player Data Flows

This doc explains runtime data flow in `proteus-lib`, from `Player` construction through decode, mix, effects, sink append, and control actions.

## Core Files (what owns what)

- `proteus-lib/src/playback/player/mod.rs`: high-level `Player` API and shared runtime state.
- `proteus-lib/src/playback/player/controls.rs`: transport/lifecycle controls (`play`, `pause`, `seek`, `stop`).
- `proteus-lib/src/playback/player/effects.rs`: DSP chain updates and meter accessors.
- `proteus-lib/src/playback/player/settings.rs`: runtime tuning knobs (buffer/fade/jitter).
- `proteus-lib/src/playback/player/runtime/thread.rs`: playback thread bootstrap.
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`: worker loop (sink/device side).
- `proteus-lib/src/playback/engine/mod.rs`: `PlayerEngine` setup and mix-thread wiring.
- `proteus-lib/src/playback/engine/mix/runner.rs`: hot mixing loop, source spawning, chunk emission.
- `proteus-lib/src/playback/engine/mix/track_mix.rs`: per-chunk track mixing.
- `proteus-lib/src/playback/engine/mix/output_stage.rs`: effect-chain output/tail handling.
- `proteus-lib/src/container/prot.rs`: container/play-settings model, selection, shuffle schedule.
- `proteus-lib/src/track/single.rs`: per-source decode thread.
- `proteus-lib/src/track/container.rs`: shared-container decode fast path.
- `proteus-lib/src/track/buffer.rs`: decoded sample push + end-of-stream signaling.

## Entry Flows: Creating a `Player`

## A) `Player::new(file_path)` (single `.prot`/`.mka`)

1. `Player::new` delegates to `Player::new_from_path_or_paths(Some(path), None)`.
2. `Prot::new(path)` is created.
3. `Prot::new` internally calls `load_play_settings()` and `refresh_tracks()`.
4. `Player` initializes shared state (`state`, `sink`, `effects`, metrics, buffer settings, etc.).
5. `Player::initialize_thread(None)` is called immediately.

Main files:
- `proteus-lib/src/playback/player/mod.rs`
- `proteus-lib/src/container/prot.rs`
- `proteus-lib/src/playback/player/runtime/thread.rs`

## B) `Player::new_from_file_paths(Vec<PathsTrack>)` (standalone files)

1. `Player::new_from_path_or_paths(None, Some(paths))`.
2. `Prot::new_from_file_paths(paths)` builds file-path dictionary and calls `refresh_tracks()`.
3. `Player` initializes runtime state.
4. `initialize_thread(None)` is called.

Main files:
- `proteus-lib/src/playback/player/mod.rs`
- `proteus-lib/src/container/prot.rs`

## C) `Player::new_from_file_paths_legacy(Vec<Vec<String>>)`

1. Legacy vectors map to `PathsTrack::new_from_file_paths`.
2. Flow then matches (B).

Main files:
- `proteus-lib/src/playback/player/mod.rs`
- `proteus-lib/src/container/prot.rs`

## What `initialize_thread()` does

1. Clears previous `finished_tracks` state.
2. Creates fresh abort flag/playback generation state and resets runtime markers.
3. Spawns detached worker thread.
4. Worker thread (`run_playback_thread`) then:
- Creates `PlayerEngine`.
- Opens default output stream.
- Replaces sink with fresh sink connected to current mixer.
- Initializes sink paused at muted volume.
- Seeds startup silence when configured.
- Starts receiving mixed chunks and appending to sink.
- Maintains playback time and output meter state.

Main files:
- `proteus-lib/src/playback/player/runtime/thread.rs`
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`
- `proteus-lib/src/playback/engine/mod.rs`

## Runtime Flow after `play()`

1. `play()` ensures a thread exists; if not, it initializes one.
2. `play()` sets player state to `Resuming`.
3. Worker loop gates start/resume until `start_sink_chunks` is satisfied.
4. Worker runs `resume_sink` (fade-in, then `sink.play()`), then transitions to `Playing`.

Main files:
- `proteus-lib/src/playback/player/controls.rs`
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`

## Engine Data Flow (decode -> buffer -> mix -> sink)

1. `PlayerEngine::start_receiver()` readies track buffers and spawns mix thread.
2. Mix thread builds runtime shuffle plan from `Prot`.
3. Source decoders are spawned:
- Container fast path: `buffer_container_tracks` when all active sources are container track IDs and no upcoming shuffle events.
- General path: `buffer_track` per active runtime source.
4. Decoder threads push interleaved `f32` samples into per-track bounded ring buffers.
5. Mix loop snapshots active buffers and mixes to premix buffer with per-track weights/pan gains.
6. Output stage runs DSP chain and effect-tail handling.
7. Processed samples are wrapped as `(SamplesBuffer, duration)` and sent over channel.
8. Worker thread appends chunk to sink and updates timing/meter state.

Main files:
- `proteus-lib/src/playback/engine/mod.rs`
- `proteus-lib/src/playback/engine/mix/runner.rs`
- `proteus-lib/src/playback/engine/mix/track_mix.rs`
- `proteus-lib/src/playback/engine/mix/output_stage.rs`
- `proteus-lib/src/track/single.rs`
- `proteus-lib/src/track/container.rs`
- `proteus-lib/src/track/buffer.rs`

## Shuffle-Point Flow (precompute + hot-loop use)

## Precompute phase (`Prot::refresh_tracks`)

For each track definition (`SettingsTrack` or `PathsTrack`):
1. Parse `shuffle_points` timestamps.
2. Build timestamp-ordered schedule (`0ms` + all shuffle points).
3. At each timestamp, produce complete current source list (randomizing only tracks that shuffle there).
4. Store schedule in `Prot.shuffle_schedule`.

## Runtime phase (mix loop)

1. At start, mix loop requests runtime plan from `Prot`:
- current sources for `start_time`
- upcoming events after `start_time`
2. Loop tracks `next_shuffle_event_index`.
3. When source timeline reaches event timestamp, only changed slots are replaced.
4. Outgoing slot keys are kept briefly in fade-out set; new slots are spawned with new keys.
5. Chunk size is clipped so one output chunk does not cross pending shuffle boundary.

Main files:
- `proteus-lib/src/container/prot.rs`
- `proteus-lib/src/playback/engine/mix/runner.rs`
- `proteus-lib/src/playback/engine/mix/track_mix.rs`

## Control Flows During Playback

## `seek(ts)`

1. Update shared timestamp.
2. Capture previous transport state and configured seek fades.
3. If currently active, fade current sink out (`seek_fade_out_ms`).
4. Request effects reset and clear pending inline-effects updates.
5. Kill current playback thread.
6. Reinitialize worker at `ts`.
7. If previously active, set next resume fade (`seek_fade_in_ms`) and transition to `Resuming`.

File:
- `proteus-lib/src/playback/player/controls.rs`

## `refresh_tracks()` / `shuffle()`

1. `Prot::refresh_tracks()` recomputes active selections and shuffle schedule.
2. Optional IR overrides are re-applied.
3. Effects reset requested; pending inline update cleared.
4. If runtime active, player seeks to current time to apply new selection.

Files:
- `proteus-lib/src/playback/player/controls.rs`
- `proteus-lib/src/container/prot.rs`

## `play_at(ts)`

1. Set timestamp.
2. Request effects reset and clear pending inline update.
3. Kill current thread.
4. Start new thread at `ts`.
5. Resume playback.

File:
- `proteus-lib/src/playback/player/controls.rs`

## State and Synchronization Model (high level)

- Thread coordination uses `Arc<Mutex<...>>` + atomics (`abort`, playback id, thread-exists, timing flags).
- Decoder/mixer backpressure uses bounded ring buffers and condvar notifications.
- Sink append path is decoupled from mix thread via channel.

Main files:
- `proteus-lib/src/playback/player/runtime/worker/runner.rs`
- `proteus-lib/src/playback/engine/mod.rs`
- `proteus-lib/src/playback/engine/mix/runner.rs`

## Mental Model Summary

- `Prot` chooses what should play and when (including shuffle schedule).
- Decode threads fill per-track ring buffers.
- Mix thread consumes those buffers, applies gain/effects, emits output chunks.
- Worker thread appends chunks to sink and manages playback time/state.
