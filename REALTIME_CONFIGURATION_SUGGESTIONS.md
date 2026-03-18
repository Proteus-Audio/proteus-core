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

For non-convolution chains, `max_sink_chunks = 2` usually means roughly a few tens of milliseconds of Proteus-managed sink backlog, not seconds of buffered audio. End-to-end control latency will still include the current in-flight chunk plus device/OS buffering.

For convolution-heavy chains, try `max_sink_chunks = 1` first. Because each chunk can already be much larger there, chunk-count-based buffering is a blunt tool and configuration alone will only get you so far until the library adds a time-based cap and smaller sink-facing output slices.

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

Today, `sink_len` is only a proxy. It is much less informative when convolution is active because one queued chunk may already represent well over 100 ms. Once `FR-03` lands, prefer queued milliseconds over chunk count.

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

## What Configuration Alone Cannot Fix

There is still a library-side gap:

- sink backpressure is controlled by chunk count, not by actual queued milliseconds
- output chunk size can become large when mix scheduling aligns to convolution batching
- the public API does not expose a direct "estimated audible control latency" value

That is why this repository should also track a follow-up feature request for bounded audible control latency during live editing.
