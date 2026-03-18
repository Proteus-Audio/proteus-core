# Realtime Configuration Suggestions

## Summary

The remaining "I turn the knob now, I hear it a second later" behavior is not coming from `parameter_ramp_ms`. That ramp defaults to 5 ms, so it can smooth clicks, but it cannot create a 1-2 second delay by itself.

What is happening now is a combination of:

- the mix thread applying control changes to future chunks only
- the playback sink potentially already holding a large backlog of pre-rendered audio

After `FR-02`, parameter changes reach the mix thread quickly. The audible lag you are hearing is mainly a queueing problem, so this is partly an editor/application configuration issue and partly a remaining library-structure issue.

## What Matters Most

### 1. Keep the sink queue shallow

`Player::set_effect_parameter()` and `Player::set_effect_enabled()` update the mix thread for newly rendered audio. They cannot modify audio that has already been appended to the output sink.

Today, `max_sink_chunks == 0` means "no sink backpressure at all". If the editor leaves that setting at `0`, the sink can get far ahead of the playback head, which turns a fast control-path update into a slow audible update.

### 2. Chunk size is already about 30 ms minimum

The mix runtime currently renders in chunks with a floor of about 30 ms. That is a reasonable realtime control granularity for most effects.

If convolution reverb is active, the runtime can force a much larger processing batch. In stereo at 48 kHz, the current preferred convolution batch is about 8192 frames, or about 171 ms of audio. That means convolution-heavy chains will feel less immediate even with good sink settings.

### 3. Startup buffering is not the same thing as control latency

`start_buffer_ms` mostly affects startup and resume behavior. It should still be kept modest in authoring mode, but the multi-second lag during playback is usually caused by sink backlog, not by the startup buffer.

## Recommended Authoring Profile

Use a dedicated low-latency authoring profile in the editor instead of reusing a "stable playback" profile.

Suggested starting point:

```rust
player.set_start_buffer_ms(20.0);
player.set_start_sink_chunks(0);
player.set_max_sink_chunks(2);
player.set_startup_silence_ms(0.0);
player.set_startup_fade_ms(80.0);
player.set_seek_fade_out_ms(20.0);
player.set_seek_fade_in_ms(50.0);
player.set_parameter_ramp_ms(5.0);
player.set_inline_effects_transition_ms(15.0);
player.set_append_jitter_log_ms(8.0);
```

Interpretation:

- `start_buffer_ms = 20`: good default for responsive editing; raise only if underruns appear
- `start_sink_chunks = 0`: do not intentionally wait for an extra queued sink backlog before playback starts
- `max_sink_chunks = 2`: keep the sink close to the playback head instead of letting it run far ahead
- `startup_silence_ms = 0`: do not add avoidable startup latency
- `parameter_ramp_ms = 5`: keep click suppression without making knob turns feel sluggish
- `append_jitter_log_ms = 8`: turn on jitter diagnostics while tuning the profile

For non-convolution chains, `max_sink_chunks = 2` usually means roughly one small handful of output chunks, not seconds of buffered audio.

For convolution-heavy chains, try `max_sink_chunks = 1` first. Because each chunk can already be much larger there, chunk-count-based buffering is a blunt tool.

## API Usage Recommendations

For live knob drags and effect toggles:

- use `set_effect_parameter()`
- use `set_effect_enabled()`

For swapping a whole chain definition without resetting transport:

- use `set_effects_inline()`

Do not use `set_effects()` for continuous editor knob motion. It intentionally requests an effect reset and a seek-to-current-time refresh, which is much heavier and does not preserve live effect state.

## Diagnostics To Watch In The Editor

While tuning your editor profile, monitor:

- `debug_sink_state()`: in authoring mode, `sink_len` should stay low, typically `0`, `1`, or `2`
- `get_dsp_metrics()`: watch `late_append_count`, `late_append_active`, `underrun_count`, and `overrun`
- `append_jitter_log_ms`: enable this temporarily to see when the worker is missing append timing targets

If you still hear very large delay while `sink_len` stays low, the next suspect is outside Proteus:

- host audio device buffer size
- OS audio stack buffering
- any extra buffering the editor adds above `Player`

## Practical Guidance

Use two runtime profiles:

- Authoring mode: low `start_buffer_ms`, very small finite `max_sink_chunks`, inline parameter updates
- Stable/export mode: larger buffers if needed for maximum dropout resistance

If the editor currently always runs with `max_sink_chunks = 0`, fixing that should be your first change. That is the most likely reason you still hear control changes 1-2 seconds late.

## What Configuration Alone Cannot Fix

There is still a library-side gap:

- sink backpressure is controlled by chunk count, not by actual queued milliseconds
- output chunk size can become large when convolution batching is active
- the public API does not expose a direct "estimated audible control latency" value

That is why this repository should also track a follow-up feature request for bounded audible control latency during live editing.
