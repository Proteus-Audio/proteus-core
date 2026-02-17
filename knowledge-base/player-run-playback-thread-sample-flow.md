# `run_playback_thread` Sample Processing Flow

This note follows one block of audio samples through `proteus-lib/src/playback/player/runtime/worker/runner.rs::run_playback_thread`, from file decode all the way to speaker output.

## High-level call chain

1. `Player::initialize_thread` spawns worker thread.
- File/function: `proteus-lib/src/playback/player/runtime/thread.rs::initialize_thread`
2. Worker starts `run_playback_thread`, creates `PlayerEngine`, opens output stream, creates sink.
- File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::run_playback_thread`
3. `PlayerEngine::start_receiver` spawns mix thread and returns `Receiver<(SamplesBuffer, f64)>`.
- File/function: `proteus-lib/src/playback/engine/mod.rs::start_receiver`
4. Mix thread spawns source decoders, mixes track buffers, runs DSP chain, emits chunk.
- File/function: `proteus-lib/src/playback/engine/mix/runner.rs::spawn_mix_thread`
5. Worker receives chunk, updates meter/timing, appends to `rodio::Sink`.
- File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::update_sink`
6. `rodio` plays sink queue through the stream mixer/device.
- File/functions: `proteus-lib/src/playback/player/runtime/worker/runner.rs::initialize_sink`, `rodio::Sink::append`

## Stage 0: Thread bootstrap and output device setup

Inside `run_playback_thread`:

- Creates `PlayerEngine` with shared state (prot, effects, metrics, reset flags, buffer settings).
- Opens default output device with retry logic.
  - File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::open_output_stream_with_retry`
- Gets `stream.mixer()` and creates a new sink connected to it.
  - File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::initialize_sink`
- Starts sink paused at muted volume (`0.0`) so startup/resume fade logic controls first audible ramp.
- Copies duration from engine and sets current playback timestamp (`time_passed`) to start timestamp.
  - File/functions: `set_duration_from_engine`, `set_start_time`
- Optionally appends startup silence pre-roll to stabilize output startup.
  - File/function: `append_startup_silence`

Important ownership detail:
- `stream` is kept alive for the whole `run_playback_thread` scope, so sink output remains routed to the active output device while the loop runs.

## Stage 1: Sample read/decode from files

The worker calls:
- `engine.start_receiver()`
  - File/function: `proteus-lib/src/playback/engine/mod.rs::start_receiver`
  - This calls `spawn_mix_thread(...)` and gives back an MPSC receiver.

Inside `spawn_mix_thread`:

- Builds runtime shuffle plan from `Prot`.
  - File/function: `proteus-lib/src/playback/engine/mix/runner.rs::spawn_mix_thread`
- Spawns decode producers in one of two ways:
1. Container fast-path (`.prot/.mka`, no upcoming shuffle events):
- `buffer_container_tracks(...)`
- File/function: `proteus-lib/src/track/container.rs::buffer_container_tracks`
2. General path (file path or per-slot shuffle source):
- `SourceSpawner::spawn(...)` -> `buffer_track(...)`
- File/functions: `proteus-lib/src/playback/engine/mix/source_spawner.rs::spawn`, `proteus-lib/src/track/single.rs::buffer_track`

Decode details:

- `buffer_track` opens media with Symphonia helper:
  - `open_file` -> `get_reader` + `get_decoder`
  - File/functions: `proteus-lib/src/tools/tools.rs::open_file`, `get_reader`, `get_decoder`
- Seeks to `start_time` (`SeekTo::Time`) then loops packets.
- Decodes packets and converts channel samples to `f32`.
  - File/function: `proteus-lib/src/track/convert.rs::process_channel`
- Converts decoded packets to interleaved stereo samples:
  - Mono duplicates channel 0 into both L/R.
  - Multi-channel decode currently uses first two channels for the interleaved output vector.
  - File/function: `proteus-lib/src/track/single.rs::buffer_track`

Container decode details:

- `buffer_container_tracks` shares one format reader across track IDs, keeps per-track decoders, decodes packets by `track_id`, then pushes samples for each selected key.
- File/function: `proteus-lib/src/track/container.rs::buffer_container_tracks`

## Stage 2: Producer ring buffers (track staging)

Decoded interleaved samples are pushed into per-track bounded ring buffers:

- File/function: `proteus-lib/src/track/buffer.rs::add_samples_to_buffer_map`
- Buffer type: `TrackBuffer = Arc<Mutex<Bounded<Vec<f32>>>>`
  - File: `proteus-lib/src/audio/buffer.rs`
- Push is blocking when full:
  - If full, decoder waits on `Condvar` (or sleeps without notify handle), then retries.
- On EOS/error completion, track key is marked finished.
  - File/function: `proteus-lib/src/track/buffer.rs::mark_track_as_finished`

## Stage 3: Track mixing into premix queue

Inside each mix-loop iteration (`spawn_mix_thread`):

- Takes snapshots of active/fading track buffers, weights, and per-channel gains.
- Waits for startup buffering target (`start_samples`) before first output.
- Computes chunk size using:
  - `min_mix_samples`
  - available per-track buffered samples
  - boundary clipping to next shuffle event timestamp
  - premix queue capacity
- Mixes samples from active and fading tracks into one interleaved chunk.
  - File/function: `proteus-lib/src/playback/engine/mix/track_mix.rs::mix_tracks_into_premix`
  - Applies:
    - track weight (`track_weights`)
    - per-channel gain/pan (`track_channel_gains`)
    - crossfade gain for outgoing tracks at shuffle boundaries
- Pushes mixed interleaved chunk into `PremixBuffer` FIFO.
  - File: `proteus-lib/src/playback/engine/premix.rs`

Timeline detail:
- `source_timeline_frames` advances by consumed source frames from track mix (not by post-DSP output), so shuffle event timing is based on source timeline.

## Stage 4: DSP/effects stage and tail management

After premix is available, output stage decides what to emit:

- File/function: `proteus-lib/src/playback/engine/mix/output_stage.rs::produce_output_samples`

Decision order:
1. If `effects_buffer` (tail queue) has samples, output tail first.
2. Else, if enough premix exists (or draining final remainder), pop chunk from `PremixBuffer` and process DSP.
3. Else emit nothing this iteration.

DSP chain:

- Runs each effect in order (`effect.process`) via `run_effect_chain`.
  - File/function: `proteus-lib/src/playback/engine/mix/effects.rs::run_effect_chain`
- Supports inline chain transition (crossfading old/new effect chains).
- Supports reset signal:
  - Calls `reset_state()` on each effect.
  - Clears tail and premix buffers.
- Drain behavior:
  - When tracks finished and premix empty, chain is called with `drain=true` to flush lingering tails (for example, reverb/convolution tails).

Length reconciliation:

- If DSP output is shorter than input chunk:
  - Appends missing dry input tail to keep cadence stable.
- If DSP output is longer than input chunk:
  - Splits extra samples into `effects_buffer` tail for later emission.

## Stage 5: Mix thread emits chunks to worker

The mix thread wraps processed `Vec<f32>` as `SamplesBuffer` and computes duration:

- File/function: `proteus-lib/src/playback/engine/mix/output_stage.rs::send_samples`
- Duration formula:
  - `length_in_seconds = samples.len() / sample_rate / channels`
- Sends `(SamplesBuffer, length_in_seconds)` over sync channel to worker thread.

## Stage 6: Worker receives chunk and appends to sink

`run_playback_thread` receives chunks with `recv_timeout(20ms)` and calls `update_sink`.

- File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::update_sink`

`update_sink` behavior:

1. Verifies worker generation (`playback_id`) is still current.
2. Applies backpressure via `wait_for_sink_capacity` (`max_sink_chunks` guard).
3. Records append timing/jitter stats (`update_append_timing`).
4. Pushes chunk samples into output meter queue (`OutputMeter::push_samples`).
  - File: `proteus-lib/src/playback/output_meter.rs`
5. Appends `SamplesBuffer` to `rodio::Sink`.
6. Stores chunk duration in `chunk_lengths` for playback-time accounting.
7. Calls `update_chunk_lengths` and `check_runtime_state` to keep time/state responsive.

## Stage 7: Playback-state gating and clock advancement

### Transport gating
- File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::check_runtime_state`

It enforces:
- Abort: fade/pause/clear sink and exit worker.
- Resume gating: wait until `sink.len() >= start_sink_chunks` before starting playback.
- Pause: fade out and move state to `Paused`.
- Resume: fade in and move state to `Playing`.

### Time/meter advancement
- File/function: `proteus-lib/src/playback/player/runtime/worker/runner.rs::update_chunk_lengths`

Clock model:
- `chunk_lengths` tracks enqueued chunk durations.
- While buffering is active, consumed chunk count is inferred from `chunk_lengths.len() - sink.len()`.
- `timer` tracks sub-chunk elapsed play time.
- `time_passed = consumed_chunk_time + timer_elapsed`.
- Meter is advanced with `delta = current_audio_time - last_meter_time`.

## Stage 8: Drain to end and thread completion

When sender side closes, worker enters a drain loop:

- Marks buffering complete and computes `final_duration`.
  - File/function: `mark_buffering_complete`
- Polls runtime state and clock until:
  - `engine.finished_buffering()` and
  - `time_passed >= final_duration` (with small epsilon)
  - File/function: `is_drain_complete`

At that point, `run_playback_thread` returns and stream/sink resources drop.

## Practical mental model

One chunk’s journey is:

1. **Decode** from container/file packets (`buffer_track` / `buffer_container_tracks`).
2. **Convert/interleave** to `Vec<f32>` stereo-like stream (`process_channel` + interleave logic).
3. **Queue** into per-track bounded ring buffers (`add_samples_to_buffer_map`).
4. **Mix** active/fading tracks into premix FIFO (`mix_tracks_into_premix`).
5. **Process DSP** chain and manage effect tails (`produce_output_samples`).
6. **Send** `(SamplesBuffer, duration)` to playback worker (`send_samples`).
7. **Append** to `rodio::Sink` (`update_sink`), then sink/mixer/output stream drive device playback.
