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

- the mix thread measures and tags each metered chunk
- the output/runtime layer advances a playback-aligned cursor
- the public “live” effect meter exposes the snapshot associated with the chunk
  that has reached audible playback time

This preserves the correct measurement location while fixing the timing
semantics.

### B. Queue effect-meter snapshots alongside output timing

The output side already has timing information about queued chunks from the
playback worker path used in [`FR-03`](./FR-03-bounded-live-effect-control-latency.md).

Build on that:

- when the mix thread publishes a metered chunk, attach:
  - per-effect snapshot
  - chunk duration / frame count
  - monotonic sequence id
- when the worker/sink bookkeeping advances playback, retire snapshot entries
  in the same order as the queued audio drains
- expose the most recent **played** effect snapshot as the audible-time meter

This is conceptually similar to a small FIFO of effect-meter snapshots aligned
to the already-queued output audio.

### C. Preserve an explicit processing-time diagnostic path

The mix-time snapshot is still useful for diagnostics and testing.

So v1 should distinguish two semantics explicitly:

- **processing-time** effect meter
- **audible-time** effect meter

Recommended API direction:

```rust
pub enum EffectMeterTiming {
    ProcessingTime,
    AudibleTime,
}
```

The CLI live playback surface should use `AudibleTime`.

Offline tooling can keep using `ProcessingTime`.

If a new public enum feels too heavy for v1, an equivalent split through two
player accessors is also acceptable.

### D. Keep backlog accounting cheap

The effect meter should not create a second independent timing system if the
worker already tracks chunk durations.

Preferred approach:

- reuse the worker-side queued-duration bookkeeping
- store one effect-meter payload per queued output chunk
- advance/destroy entries as chunks are known to have drained from the Proteus
  managed queue

This should remain bounded by the same sink-latency envelope discussed in
`FR-03`, so memory growth is naturally limited.

### E. Be explicit about the final accuracy boundary

Proteus can align to the point where audio has drained from its managed sink
queue.

It cannot perfectly know the final DAC emission time for every backend.

So the v1 contract should be explicit:

- effect meters are aligned to the library-managed audible playback boundary
- small residual device/backend latency beyond Proteus may still remain

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
