# FR-05: Audible-Time Aligned Effect Metering

## Summary

The current per-effect metering added by [`FR-01`](./FR-01-per-effect-level-metering.md)
and surfaced in the CLI by [`FR-04`](./FR-04-cli-effect-metering-harness.md)
is currently **mix-time aligned**, not **audible-time aligned**.

That means the reported per-effect input/output levels describe the chunk that
the mix thread has most recently processed, not the chunk the listener is
hearing at that same moment.

This becomes visible as soon as the CLI shows live per-effect meters during
normal playback: the effect meters can lead the actual audio because of startup
buffering, sink queue backlog, output slicing, and general output-path latency.

The library needs a second-stage timing model so effect metering shown to users
tracks **playback time**, not merely **processing time**.

---

## Motivation

For offline inspection, mix-time metering is acceptable:

- it is deterministic
- there is no live output sink
- “latest processed chunk” is the whole point

For live playback, that is the wrong semantic.

When a user watches the effect meters while listening, the natural expectation
is:

- the displayed effect levels correspond to the audio currently being heard

Today that expectation is not met.

The output side may have:

- startup buffering
- queued sink chunks
- authoring-mode output slicing
- device / backend latency after Proteus has appended the chunk

So the mix thread can be significantly ahead of the listener. The effect meter
therefore shows the future, not the present.

---

## Current Behavior

### Where the snapshot is taken

The current effect-meter snapshots are captured around
[`run_effect_chain()`](../proteus-lib/src/playback/engine/mix/effects.rs)
and published from the mix runtime in
[`effect_metering.rs`](../proteus-lib/src/playback/engine/mix/runner/effect_metering.rs).

That is the correct place to measure per-effect boundaries, but it only gives us
the meter values for the chunk currently being rendered.

### Why that is not enough

After the chunk is processed, it still has to travel through the output path:

1. mix thread renders the chunk
2. chunk is sent to the playback worker
3. worker appends it to the sink
4. queued sink backlog drains over time
5. the backend/device actually plays the audio

The current effect meter is published at step 1, but the user cares about step
4 or 5.

### Symptoms

- effect meters visibly lead the heard audio
- effect changes appear in the meter before they become audible
- the mismatch grows with larger sink backlog or startup buffering
- the issue is more obvious on effect-heavy or latency-managed chains

---

## Relationship To FR-03

[`FR-03`](./FR-03-bounded-live-effect-control-latency.md) reduced and exposed
queued-output latency, but it did not change the semantics of effect-meter
publication.

That means:

- `FR-03` helps reduce the size of the mismatch
- this FR fixes the fact that the mismatch exists at all

Even with a bounded latency budget, the meter should still be aligned to
audible playback time rather than mix-thread processing time.

---

## Desired Outcome

The public/runtime effect meter should support an **audible-time aligned view**
for live playback surfaces such as the CLI TUI and future GUI clients.

When a caller polls the effect levels during playback, the returned snapshot
should correspond as closely as practical to the chunk currently being heard,
not simply the last chunk rendered by the mix thread.

---

## Proposed Design

### A. Split meter production from meter presentation

Keep the existing mix-thread boundary measurement as the producer stage.

Do **not** move the actual measurement point out of the effect chain.

Instead, introduce a second stage:

- the mix thread measures and tags each metered snapshot with a playback-clock
  timestamp (the `time_chunks_passed` value at the point the corresponding
  audio enters the worker queue)
- the output/runtime layer advances a playback-aligned cursor as chunks drain
- the public “live” effect meter exposes the snapshot whose timestamp is at or
  just before the current audible playback time

This preserves the correct measurement location while fixing the timing
semantics.

### B. Timestamp-indexed snapshot ring buffer

#### Why not one snapshot per output chunk

The current metering is **rate-limited**: `MixEffectMeteringState` accumulates
rendered frames and only emits a snapshot when the accumulated count reaches the
refresh interval (default 30 Hz / ~1600 frames at 48 kHz). Many output chunks
pass without producing a snapshot, and a single snapshot may span multiple
output chunks.

Additionally, when `output_slice_ms` is active (FR-03 Section C), a single DSP
pass is sliced into several smaller output chunks — but the effect measurement
happens once per DSP pass, not per slice.

This means a 1:1 “one snapshot per queued output chunk” model does not work.

#### Proposed model: timestamped ring buffer

Instead of trying to pair snapshots with individual chunks, use a small
**bounded ring buffer** of timestamped snapshots:

1. When `ChunkEffectMetering::finish()` publishes a snapshot, it also records
   the current **playback-clock position** of that audio. This is the
   cumulative duration of audio sent to the worker at the time the snapshot's
   chunk crosses the mix→worker boundary.

2. The ring buffer lives in `EffectMeter` (or a new sibling struct) and is
   written by the mix thread. It should use a lock-free structure (e.g.,
   `crossbeam::ArrayQueue` or a single-producer/single-consumer ring) to avoid
   blocking the mix thread. The current `try_lock()` approach works for
   “latest only” but is unsuitable for a queue where dropped entries create
   timing gaps.

3. On the consumer side, the caller reads the current audible playback time
   (derived from `time_chunks_passed` in `timing.rs`) and finds the most
   recent snapshot whose timestamp ≤ that time.

4. Snapshots older than the current audible time can be retired (popped from
   the front of the ring buffer).

#### Capacity bound

The ring buffer capacity should be bounded explicitly, not just implicitly by
the sink-latency envelope:

- With `max_sink_latency_ms = 60` and snapshots at 30 Hz, only ~2 snapshots
  will be queued at any time — very small.
- Without a latency budget (default playback), startup buffering could push the
  mix thread significantly ahead. A fixed capacity of 16–32 entries is more
  than sufficient and prevents unbounded growth regardless of configuration.
- When the buffer is full, the oldest entry is overwritten (ring semantics).

### C. Preserve an explicit processing-time diagnostic path

The mix-time snapshot is still useful for diagnostics and testing.

So v1 should distinguish two semantics explicitly:

- **processing-time** effect meter — the latest snapshot from the mix thread
  (current behavior, unchanged)
- **audible-time** effect meter — the snapshot corresponding to the chunk
  currently draining from the managed sink queue

Recommended API direction for v1: **two separate player accessors** rather than
a timing enum parameter:

```rust
// Existing — unchanged, returns the latest mix-thread snapshot
pub fn effect_levels(&self) -> Option<Vec<EffectLevelSnapshot>>;

// New — returns the snapshot aligned to current audible playback time
pub fn effect_levels_audible(&self) -> Option<Vec<EffectLevelSnapshot>>;
```

This avoids adding a parameter to every metering call and keeps the two
internal paths cleanly separated. The existing `effect_levels()` continues to
work for offline tooling; the new `effect_levels_audible()` is what live
playback surfaces use.

A unifying `EffectMeterTiming` enum can be introduced later if more timing
modes are needed.

### D. Keep backlog accounting cheap

The effect meter should not create a second independent timing system.

Preferred approach:

- reuse the worker-side `time_chunks_passed` bookkeeping already maintained in
  `timing.rs` as the authoritative audible-time clock
- expose this clock (or a snapshot of it) through the player API so the
  consumer can compare it against snapshot timestamps
- the ring buffer lookup is O(n) on a very small n (typically 1–3 entries);
  a linear scan from the tail is sufficient

The worker already computes `queued_sink_ms` as part of
`publish_output_latency_metrics()`. This value, combined with the total
elapsed mix time, gives the audible-time boundary without new bookkeeping.

### E. Address the `try_lock` publication path

The current `try_publish_levels()` uses `Mutex::try_lock()` and silently drops
the snapshot if the consumer holds the lock. For “latest only” this is fine —
the next snapshot replaces it anyway.

For the audible-time ring buffer, dropped entries leave timing gaps. Two
options:

1. **Lock-free queue** (preferred): Use a bounded SPSC ring buffer
   (`crossbeam::ArrayQueue` or a hand-rolled atomic ring). The mix thread
   always succeeds in pushing; if the buffer is full, the oldest entry is
   overwritten. The consumer pops entries up to the current audible time.

2. **Fallback — keep `try_lock` with a `VecDeque`**: The consumer holds the
   lock only briefly to clone and drain. In practice the contention window is
   tiny (sub-microsecond clone of a small vec). If a snapshot is dropped, the
   audible-time view holds the previous value — acceptable degradation, not a
   correctness failure.

Option 1 is cleaner but adds a dependency. Option 2 is pragmatic for v1 and
can be upgraded later.

### F. Be explicit about the final accuracy boundary

Proteus can align to the point where audio has drained from its managed sink
queue.

It cannot perfectly know the final DAC emission time for every backend.

So the v1 contract should be explicit:

- effect meters are aligned to the library-managed audible playback boundary
- small residual device/backend latency beyond Proteus may still remain

The audible-time meter also inherits the refresh-rate quantization of the
snapshot producer (e.g., 33 ms at 30 Hz). This is acceptable for visual
metering — the meter is aligned to within one refresh period of the true
audible boundary, which is well below perceptual thresholds.

That is still a large improvement over the current “latest processed chunk”
semantics.

---

## Files Likely Affected

| File | Why |
| --- | --- |
| `proteus-lib/src/playback/effect_meter.rs` | Add queued/live snapshot staging rather than one latest-only store |
| `proteus-lib/src/playback/engine/mix/runner/effect_metering.rs` | Tag and publish per-chunk meter snapshots into a timing-aware queue |
| `proteus-lib/src/playback/player/runtime/worker/timing.rs` | Align queued chunk drain bookkeeping with effect-meter snapshot retirement |
| `proteus-lib/src/playback/player/runtime/worker/sink.rs` | Advance the audible-time meter as queued output drains |
| `proteus-lib/src/playback/player/metering.rs` | Expose audible-time vs processing-time access semantics |
| `proteus-cli/src/cli/playback_runner.rs` | Switch live playback UI to audible-time effect meters |
| `proteus-cli/src/cli/ui.rs` | Render the corrected live meter semantics |

---

## Acceptance Criteria

- [ ] live playback effect meters no longer lead the heard audio by the full
      Proteus-managed sink backlog
- [ ] the live CLI effect-meter pane reflects audible playback time rather than
      last-processed mix time
- [ ] offline metering/reporting can still use processing-time snapshots for
      deterministic inspection
- [ ] effect meter timing remains bounded and coherent when authoring-mode sink
      slicing or sink backlog limits are active
- [ ] the public/runtime API makes the timing semantics explicit enough that
      callers do not confuse processing-time and audible-time meters

## Status

Open.
