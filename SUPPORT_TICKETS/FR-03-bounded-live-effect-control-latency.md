# FR-03: Bounded Audible Control Latency for Live Effect Editing

## Summary

`FR-02` fixed the quality of inline effect changes: parameter updates are now smoothed and click-free. It did not yet guarantee when those changes become audible.

In the current playback architecture, a parameter change can reach the mix thread quickly and still be heard noticeably later because the output side may already have a backlog of previously rendered audio queued in the sink. For authoring workflows, the library needs an explicit control-latency contract, not just artifact-free parameter ramps.

Any solution here must remain opt-in. The editor wants bounded live-edit response, but player applications still need the current stability-first behavior and should not pay extra wakeup, slicing, or buffering costs unless they explicitly choose that tradeoff.

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

The library should support a deliberate low-latency authoring mode where effect parameter changes become audible within a bounded, measurable time budget, while preserving the current stable/high-buffer behavior as the default for non-authoring use cases.

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
- if both a time budget and `max_sink_chunks` are configured, the stricter effective cap should win
- the setting should remain disabled by default
- the time-based budget should be the preferred authoring control because it remains meaningful when chunk sizes vary by effect chain

### B. Expose queued-output diagnostics publicly

Add public diagnostics that let the editor reason about responsiveness directly, for example:

- queued sink audio in milliseconds managed by Proteus
- current output append chunk duration in milliseconds
- estimated library-side control-to-audible latency budget

Without this, the editor can only inspect `sink_len`, which is not enough when one chunk may represent 30 ms in one chain and ~171 ms in another.

These diagnostics should be cheap to poll and available in normal builds. The editor should not need debug-only plumbing just to know whether authoring mode is meeting its target.

### C. Decouple internal DSP batch size from audible output slice size

In authoring mode, large internal processing batches should not automatically force equally large sink appends.

This matters most for convolution reverb:

- internal FFT/convolution batching may still want large blocks for efficiency
- audible output to the sink should be sliced into smaller chunks suitable for live editing

The library should support smaller output append slices even when an effect internally processes a larger block. This is the structural change needed to make convolution-heavy chains feel more like an editor and less like an offline render queue.

The current coupling appears to live primarily in mix-runner scheduling and chunk emission, not necessarily in convolution correctness itself. Implementation should start there. Also, smaller output slices increase send/append frequency, so they should be explicitly enabled for authoring mode rather than made universal.

### D. Add an explicit low-latency authoring profile/helper

The public API should make the intended mode obvious. Examples:

- `player.configure_for_live_authoring()`
- `PlaybackBufferSettings::live_authoring()`
- `PlaybackMode::Authoring`
- `set_target_control_latency_ms(...)`

A first step now exists via `player.configure_for_live_authoring()` / `PlaybackBufferSettings::live_authoring()`, which provide an opt-in low-latency baseline without changing defaults. This FR is still needed because a profile helper alone does not create a bounded latency contract.

The helper/profile should apply or expose a cohesive set of controls for:

- low startup buffer
- minimal start sink gate
- finite sink latency cap
- short parameter ramps
- short inline chain transition time

### E. Optional stretch: active backlog reduction on control changes

If the sink backlog exceeds the configured authoring budget, consider an opt-in strategy that reduces queued audio more aggressively when an inline effect change arrives.

This is harder than passive backpressure because already-appended audio cannot be edited in place. Any implementation would need to preserve continuity and avoid audible transport artifacts. Treat this as a separate follow-up only if a time-based cap plus smaller output slices still does not meet the authoring target, and keep it disabled by default.

---

## Files Likely Affected

| File | Why |
|---|---|
| `proteus-lib/src/playback/engine/state.rs` | Add time-based latency budget fields and profile defaults |
| `proteus-lib/src/playback/player/settings.rs` | Add public setters/helpers and latency diagnostics accessors |
| `proteus-lib/src/playback/player/runtime/worker/sink.rs` | Enforce queued-output time budget and surface diagnostics |
| `proteus-lib/src/playback/player/runtime/worker/timing.rs` | Reuse tracked chunk-duration bookkeeping for latency estimation |
| `proteus-lib/src/playback/engine/mix/runner/startup.rs` | Compute internal batch size separately from authoring-facing output slice size |
| `proteus-lib/src/playback/engine/mix/runner/state.rs` | Carry separate internal-batch and sink-slice settings through the runtime |
| `proteus-lib/src/playback/engine/mix/runner/loop_body.rs` | Emit smaller sink-facing slices without changing internal DSP batching |

---

## Acceptance Criteria

- [ ] The library exposes an opt-in time-based queued-output limit suitable for live authoring
- [ ] The editor can query queued output latency in milliseconds through the public API without relying on debug-only plumbing
- [ ] If both chunk-count and time-based sink limits are configured, the stricter cap wins
- [ ] Inline `set_effect_parameter()` / `set_effect_enabled()` changes become audible within the configured output-latency budget on non-convolution chains, absent device/OS buffering outside Proteus
- [ ] Convolution-enabled chains can opt into smaller sink append slices without forcing that behavior on stability-first playback modes
- [ ] Existing default behavior remains the default for higher-buffer, stability-first playback modes

## Status

Open.
