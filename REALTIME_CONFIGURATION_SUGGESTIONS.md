# Realtime Configuration Suggestions

## Summary

The remaining "I turn the knob now, I hear it a second later" behavior is not coming from `parameter_ramp_ms`. That ramp defaults to 5 ms, so it can smooth clicks, but it cannot create a 1-2 second delay by itself.

What is happening now is a combination of:

- the mix thread applying control changes to future chunks only
- the playback sink potentially already holding a large backlog of pre-rendered audio

After `FR-02`, parameter changes reach the mix thread quickly. The audible lag you are hearing is mainly a queueing problem, so this is partly an editor/application configuration issue and partly a remaining library-structure issue.

That does not mean the library defaults should change globally. The editor and the player applications want different tradeoffs, so low-latency settings should stay opt-in.

## What Matters Most

### 1. Keep the sink queue shallow

`Player::set_effect_parameter()` and `Player::set_effect_enabled()` update the mix thread for newly rendered audio. They cannot modify audio that has already been appended to the output sink.

Today, `max_sink_chunks == 0` means "no sink backpressure at all". If the editor leaves that setting at `0`, the sink can get far ahead of the playback head, which turns a fast control-path update into a slow audible update.

There is also a natural backpressure point between the mix thread and the playback worker: the bounded `sync_channel(1)` that carries rendered chunks. This channel has a capacity of one, so the mix thread blocks after sending a chunk until the worker consumes it and appends it to the sink. This means the mix thread can never be more than one chunk ahead of the worker — but with `max_sink_chunks = 0`, the worker immediately appends without waiting, so audio still accumulates in the sink itself.

### 2. Chunk size is already about 30 ms minimum

The mix runtime currently renders in chunks with a floor of about 30 ms. That is a reasonable realtime control granularity for most effects.

If convolution reverb is active, the runtime currently aligns mix scheduling and emitted chunk size to a much larger preferred batch. In stereo at 48 kHz, that is about 8192 frames, or about 171 ms of audio. That means convolution-heavy chains will feel less immediate even with good sink settings until the library separates internal batching from sink-facing output slices.

### 3. Startup buffering is not the same thing as control latency

`start_buffer_ms` mostly affects startup and resume behavior. It should still be kept modest in authoring mode, but the multi-second lag during playback is usually caused by sink backlog, not by the startup buffer.

## Recommended Authoring Profile

Use a dedicated low-latency authoring profile in the editor instead of reusing a "stable playback" profile.

Suggested starting point:

```rust
player.configure_for_live_authoring();

// Optional: enable diagnostics temporarily while tuning the editor profile.
player.update_buffer_settings(|settings| {
    settings.append_jitter_log_ms = 8.0;
});
```

Interpretation:

- `start_buffer_ms = 20`: good default for responsive editing; raise only if underruns appear
- `start_sink_chunks = 1`: keep the startup/resume gate minimal without intentionally pre-queueing several chunks
- `max_sink_chunks = 2`: good first-pass cap; keep the sink close to the playback head instead of letting it run far ahead
- `startup_silence_ms = 0`: do not add avoidable startup latency
- `startup_fade_ms` / `seek_fade_*_ms`: these affect transport responsiveness, not continuous knob latency, but shorter values make editing workflows feel less sluggish
- `parameter_ramp_ms = 5`: keep click suppression without making knob turns feel sluggish
- `append_jitter_log_ms = 8`: temporary tuning aid only; leave it at `0` in normal operation
- `max_sink_latency_ms = 60`: time-based sink budget in milliseconds; keeps queued output under a fixed latency target regardless of chunk size
- `output_slice_ms = 30`: post-DSP output is sliced into ~30 ms chunks before sending to the worker, so convolution-heavy chains get the same sink granularity as non-convolution chains

For non-convolution chains, `max_sink_chunks = 2` usually means roughly a few tens of milliseconds of Proteus-managed sink backlog, not seconds of buffered audio. The `max_sink_latency_ms = 60` budget provides a more reliable cap. End-to-end control latency will still include the current in-flight chunk plus device/OS buffering.

For convolution-heavy chains, the combination of `output_slice_ms = 30` and `max_sink_latency_ms = 60` now provides bounded latency. Output slicing breaks the large convolution batch (~171 ms at stereo 48 kHz) into ~30 ms slices before they reach the sink, and the time-based budget prevents the sink from accumulating more than 60 ms of queued audio. Without output slicing, chunk-count-based backpressure is too coarse: even `max_sink_chunks = 1` allows up to ~171 ms of queued audio in the sink plus up to one more chunk in the mix-to-worker channel.

## API Usage Recommendations

For live knob drags and effect toggles:

- use `set_effect_parameter()`
- use `set_effect_enabled()`

For swapping a whole chain definition without resetting transport:

- use `set_effects_inline()`

Use `set_effects()` only when you intentionally want the heavier behavior: effect-state reset, tail clear, and a seek-to-current-time refresh. Do not use it for continuous editor knob motion.

## Diagnostics To Watch In The Editor

While tuning your editor profile, monitor:

- `debug_sink_state()`: in authoring mode, `sink_len` should stay low, typically `0`, `1`, or `2`
- `get_dsp_metrics()`: watch `late_append_count`, `late_append_active`, `underrun_count`, and `overrun`
- `append_jitter_log_ms`: enable this temporarily to see when the worker is missing append timing targets

Today, `sink_len` is only a proxy. It is much less informative when convolution is active because one queued chunk may already represent well over 100 ms. With `FR-03` landed, prefer `queued_sink_ms` and `output_chunk_ms` from `get_dsp_metrics()` over chunk count.

If you still hear very large delay while `sink_len` stays low, the next suspect is outside Proteus:

- host audio device buffer size
- OS audio stack buffering
- any extra buffering the editor adds above `Player`

## Practical Guidance

Use two runtime profiles:

- Authoring mode: low `start_buffer_ms`, very small finite `max_sink_chunks`, inline parameter updates
- Stable/export mode: larger buffers if needed for maximum dropout resistance

Treat authoring mode as an application mode or explicit configuration choice, not as a new library default.

If the editor currently always runs with `max_sink_chunks = 0`, fixing that should be your first change. That is the most likely reason you still hear control changes 1-2 seconds late.

## What FR-03 Added

The following library-side gaps have been addressed:

- **Time-based sink backpressure** (`max_sink_latency_ms`): the playback worker now blocks the producer when queued output exceeds a millisecond budget, orthogonal to the chunk-count limit
- **Output slicing** (`output_slice_ms`): post-DSP output can be sliced into smaller chunks before being sent to the worker thread, decoupling internal convolution batch size from sink append granularity
- **Queued-output diagnostics** (`queued_sink_ms`, `output_chunk_ms`): available through `get_dsp_metrics()` in normal builds

The `live_authoring()` profile now sets `max_sink_latency_ms = 60 ms` and `output_slice_ms = 30 ms` by default. These settings can also be configured individually via `set_max_sink_latency_ms()` and `set_output_slice_ms()`.
