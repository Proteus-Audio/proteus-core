# FR-03: Bounded Audible Control Latency for Live Effect Editing

## Summary

`FR-02` fixed the quality of inline effect changes: parameter updates are now smoothed and click-free. It did not yet guarantee when those changes become audible.

In the current playback architecture, a parameter change can reach the mix thread quickly and still be heard noticeably later because the output side may already have a backlog of previously rendered audio queued in the sink. For authoring workflows, the library needs an explicit control-latency contract, not just artifact-free parameter ramps.

---

## Motivation

The editor workflow is DAW-like: users move a gain, pan, filter, or compressor control while listening and expect the result to follow essentially immediately.

Today there are two distinct latency domains:

1. **Control-path latency**: how quickly a parameter update reaches the mix thread. `FR-02` improved this path.
2. **Audible output latency**: how much already-rendered audio is sitting ahead of the listener in the output pipeline. This path is still weakly bounded.

When users report "I boosted gain by 7 dB, but I heard it 1-2 seconds later", that is now mostly an output-latency problem, not a smoothing problem.

---

## Current Behavior

### Control updates are fast, but only affect future output

`Player::set_effect_parameter()` and `Player::set_effect_enabled()` enqueue a targeted command for the mix thread. The mix runtime drains those commands promptly and applies them to the local effect chain before rendering the next chunk.

That is the correct behavior for realtime DSP, but it only changes audio that has not been rendered yet.

### The sink queue can still get too far ahead

The playback worker appends mixed chunks into the output sink and only applies backpressure when `max_sink_chunks > 0`. The default remains `0`, which disables that guard entirely.

That means the sink can accumulate an arbitrary amount of already-rendered audio. Any inline effect change must wait for that backlog to play out before the listener can hear it.

### Chunk count is the wrong unit for a latency budget

The current backpressure control is `max_sink_chunks`, but chunk duration is not constant:

- ordinary chains render at roughly a 30 ms minimum chunk size
- convolution reverb can force a much larger batch size

For stereo 48 kHz playback, the current convolution preferred batch is about 8192 frames, or about 171 ms of audio. A chunk-count cap is therefore too coarse to represent an authoring latency target.

### The runtime does not expose audible-latency diagnostics

The library exposes `sink_len` and DSP metrics, but not a direct "queued output milliseconds" or "estimated audible control latency" value. This makes the editor guess at why live effect updates feel late.

---

## Desired Outcome

The library should support a deliberate low-latency authoring mode where effect parameter changes become audible within a bounded, measurable time budget, while preserving the current stable/high-buffer behavior for non-authoring use cases.

---

## Proposed Design

### A. Add a time-based output latency budget

Introduce a new buffering control expressed in milliseconds rather than chunk count, for example:

```rust
pub struct PlaybackBufferSettings {
    // existing fields...
    pub max_sink_latency_ms: Option<f32>,
}
```

Behavior:

- if configured, the playback worker uses tracked queued chunk durations to keep the sink from getting further ahead than the requested time budget
- this should coexist with the existing chunk-count settings for backward compatibility
- the time-based budget should be the preferred authoring control because it remains meaningful when chunk sizes vary by effect chain

### B. Expose queued-output diagnostics publicly

Add public diagnostics that let the editor reason about responsiveness directly, for example:

- queued sink audio in milliseconds
- current output append chunk duration in milliseconds
- estimated control-to-audible latency budget

Without this, the editor can only inspect `sink_len`, which is not enough when one chunk may represent 30 ms in one chain and ~171 ms in another.

### C. Decouple internal DSP batch size from audible output slice size

Large internal processing batches should not automatically force equally large sink appends.

This matters most for convolution reverb:

- internal FFT/convolution batching may still want large blocks for efficiency
- audible output to the sink should be sliced into smaller chunks suitable for live editing

The library should support smaller output append slices even when an effect internally processes a larger block. This is the structural change needed to make convolution-heavy chains feel more like an editor and less like an offline render queue.

### D. Add an explicit low-latency authoring profile/helper

The public API should make the intended mode obvious. Examples:

- `player.configure_for_live_authoring()`
- `PlaybackMode::Authoring`
- `set_target_control_latency_ms(...)`

This helper can apply a cohesive profile for:

- low startup buffer
- zero or minimal start sink gate
- finite sink latency cap
- short parameter ramps
- short inline chain transition time

### E. Optional stretch: active backlog reduction on control changes

If the sink backlog exceeds the configured authoring budget, consider an opt-in strategy that reduces queued audio more aggressively when an inline effect change arrives.

This is harder than passive backpressure because already-appended audio cannot be edited in place. Any implementation would need to preserve continuity and avoid audible transport artifacts. Treat this as a follow-up only if a time-based cap plus smaller output slices still does not meet the authoring target.

---

## Files Likely Affected

| File | Why |
|---|---|
| `proteus-lib/src/playback/engine/state.rs` | Add time-based latency budget fields |
| `proteus-lib/src/playback/player/settings.rs` | Add public setters/helpers for authoring mode |
| `proteus-lib/src/playback/player/runtime/worker/sink.rs` | Enforce queued-output time budget and surface diagnostics |
| `proteus-lib/src/playback/player/runtime/worker/timing.rs` | Reuse tracked chunk-duration bookkeeping for latency estimation |
| `proteus-lib/src/playback/engine/mix/runner/startup.rs` | Reduce or decouple output slice size from internal processing batch size |
| `proteus-lib/src/dsp/effects/convolution_reverb/*` | Preserve convolution correctness while allowing smaller sink-facing output slices |

---

## Acceptance Criteria

- [ ] The library exposes a time-based queued-output limit suitable for live authoring
- [ ] The editor can query queued output latency in milliseconds through the public API
- [ ] Inline `set_effect_parameter()` / `set_effect_enabled()` changes become audible within the configured output-latency budget on non-convolution chains, absent device/OS buffering outside Proteus
- [ ] Convolution-enabled chains no longer force overly large sink append slices solely because of internal batch size
- [ ] Existing default behavior remains available for higher-buffer, stability-first playback modes

## Status

Open.
