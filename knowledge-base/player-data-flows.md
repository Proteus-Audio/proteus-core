# Player Data Flows

This doc explains the runtime data flows in `proteus-lib`, starting from `Player` construction and continuing through `play()`, buffering, mixing, effects, and track refresh/shuffle.

## Core Files (what owns what)

- `proteus-lib/src/playback/player.rs`: high-level `Player` API, playback state machine, sink/thread lifecycle.
- `proteus-lib/src/playback/engine/mod.rs`: `PlayerEngine` setup, ring buffer allocation, mix thread startup.
- `proteus-lib/src/playback/engine/mix.rs`: hot mixing loop, track decoder spawning, effect chain, chunk emission.
- `proteus-lib/src/container/prot.rs`: container/play-settings model, track selection, shuffle schedule precompute.
- `proteus-lib/src/container/play_settings/*.rs`: parsed `play_settings.json` schemas (`Legacy`, `V1`, `V2`, `V3`).
- `proteus-lib/src/track/single.rs`: per-track decode/buffer thread.
- `proteus-lib/src/track/container.rs`: shared-container decode/buffer thread (used in non-shuffle container fast path).
- `proteus-lib/src/track/convert.rs`: shared decode sample conversion helpers.
- `proteus-lib/src/track/buffer.rs`: push decoded samples into bounded ring buffers.
- `proteus-lib/src/playback/output_meter.rs`: output metering state.
- `proteus-lib/src/dsp/effects/*`: effect implementations; called by the mix loop.

## Entry Flows: Creating a `Player`

## A) `Player::new(file_path)` (single `.prot`/`.mka`)

1. `Player::new` delegates to `Player::new_from_path_or_paths(Some(path), None)`.
2. `Prot::new(path)` is created.
3. `Prot::load_play_settings()` parses `play_settings.json` attachment (if present), effects, and IR settings.
4. `Prot::refresh_tracks()` builds the active selection and precomputes shuffle schedule.
5. `Player` initializes shared runtime state (`state`, `sink`, `effects`, metrics, buffer settings, etc.).
6. `Player::initialize_thread(None)` is called immediately (thread/sink/engine start path is armed).

Main files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/container/prot.rs`

## B) `Player::new_from_file_paths(Vec<PathsTrack>)` (standalone files)

1. `Player::new_from_path_or_paths(None, Some(paths))`.
2. `Prot::new_from_file_paths(paths)` builds a dictionary of unique file paths.
3. `Prot::refresh_tracks()` chooses initial track paths and precomputes shuffle schedule for path-based tracks.
4. `Player` runtime state is initialized.
5. `initialize_thread(None)` is called.

Main files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/container/prot.rs`

## C) `Player::new_from_file_paths_legacy(Vec<Vec<String>>)`

1. Legacy path arrays are mapped to `PathsTrack::new_from_file_paths`.
2. Flow then matches (B).

Main files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/container/prot.rs`

## What `initialize_thread()` does

1. Clears previous `finished_tracks` state.
2. Creates a fresh `abort` flag and playback id.
3. Spawns the playback management thread.
4. Inside that thread:
- Creates `PlayerEngine` with `start_time`.
- Opens default output stream.
- Replaces sink with a fresh connected sink and pauses initially.
- Pulls duration from engine/prot.
- Starts receiving mixed chunks from engine receiver and appending to sink.
- Maintains playback time accounting and output meter.

Main files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/playback/engine/mod.rs`

## Runtime Flow after `play()`

`play()` itself sets/maintains state and resumes playback logic; most heavy work is already running in the spawned thread.

1. `play()` ensures thread exists; if not, reinitializes it.
2. Sets state to `Resuming`.
3. Management loop observes state and calls `sink.play()` once startup conditions are met.
4. Audio becomes audible when enough chunks are buffered (`start_sink_chunks`, startup settings).

Main files:
- `proteus-lib/src/playback/player.rs`

## Engine Data Flow (decode -> buffer -> mix -> sink)

1. `PlayerEngine::start_receiver()` readies ring buffers and starts mix thread.
2. Mix thread obtains runtime shuffle plan from `Prot`.
3. Decoder threads are spawned:
- Container fast path: `buffer_container_tracks` for track-id selections when there are no pending shuffle events.
- Dynamic path: `buffer_track` per active source (track id or file path), including shuffle replacements.
4. Decoder threads push interleaved samples to per-track ring buffers.
5. Mix loop waits for enough buffered samples, reads from active track buffers, applies per-track gains/weights.
6. Mix loop runs effect chain (`AudioEffect::process`) and handles effect tail buffering.
7. Mixed chunk (`SamplesBuffer`, duration) is sent over channel to player thread.
8. Player thread appends chunk to sink and updates timing/meter state.

Main files:
- `proteus-lib/src/playback/engine/mod.rs`
- `proteus-lib/src/playback/engine/mix.rs`
- `proteus-lib/src/track/single.rs`
- `proteus-lib/src/track/container.rs`
- `proteus-lib/src/track/buffer.rs`

## Shuffle-Point Flow (precompute + hot-loop use)

## Precompute phase (`Prot::refresh_tracks`)

For each track definition (`SettingsTrack` or `PathsTrack`):
1. Parse `shuffle_points` timestamps.
2. Build a timestamp-ordered schedule (`0ms` + all shuffle points).
3. At each schedule point, produce the full current selected source list (randomizing only tracks with that shuffle point).
4. Store schedule in `Prot.shuffle_schedule`.

## Runtime phase (mix loop)

1. At thread start, mix loop asks `Prot` for a runtime plan:
- current sources for `start_time`
- upcoming events after `start_time`
2. Loop keeps `next_shuffle_event_index` cursor.
3. Each iteration compares current playback timestamp with the next event timestamp.
4. When due, only changed slots are replaced with new decoder tracks/keys.
5. Chunk size is clipped so output does not cross a pending shuffle timestamp in one chunk.

Main files:
- `proteus-lib/src/container/prot.rs`
- `proteus-lib/src/playback/engine/mix.rs`

## Control Flows During Playback

## `seek(ts)`

1. Update shared timestamp.
2. Request effects reset.
3. Kill current playback thread.
4. Reinitialize thread at `ts`.
5. Resume if previous state was playing.

File:
- `proteus-lib/src/playback/player.rs`

## `refresh_tracks()` / `shuffle()`

1. `Prot::refresh_tracks()` recomputes active selections and shuffle schedule.
2. Optional IR overrides are re-applied.
3. Effects reset requested.
4. If currently active, player seeks to current time so new selection takes effect.

Files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/container/prot.rs`

## `play_at(ts)`

1. Set timestamp.
2. Effects reset.
3. Kill current thread.
4. Start thread at `ts`.
5. Resume playback.

File:
- `proteus-lib/src/playback/player.rs`

## State and Synchronization Model (high level)

- Thread coordination uses `Arc<Mutex<...>>` and atomics (`abort`, playback id, thread-exists flags).
- Decoder/mixer backpressure uses bounded ring buffers + condvar notifications.
- Sink append path is decoupled via channel from mix thread.

Main files:
- `proteus-lib/src/playback/player.rs`
- `proteus-lib/src/playback/engine/mod.rs`
- `proteus-lib/src/playback/engine/mix.rs`

## Mental Model Summary

`Player` is the orchestrator.

- `Prot` chooses what should play (including timestamped reshuffle schedule).
- Track threads decode source audio into per-track ring buffers.
- Mix thread consumes those buffers, applies gain/effects, and emits output chunks.
- Player thread appends chunks to sink and manages user-visible playback state/time.
