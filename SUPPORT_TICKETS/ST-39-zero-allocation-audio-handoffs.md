# ST-39: Benchmark and Reduce Audio Handoff Allocation Overhead

## Inspiration

Burn's article ["5x Faster than Rust Standard Channel: Optimizing Burn Communication Layer"](https://burn.dev/blog/faster-channel/)
shows that its channel regression was primarily caused by allocation pressure,
not by message passing alone. Burn improved throughput by inlining small tasks,
using preallocated arenas for larger tasks, swapping producer batches into a
private consumer buffer, and benchmarking both the isolated channel and the
real workload.

Proteus has a different workload: its hot-path channel messages own PCM
`Vec<f32>` buffers instead of boxed closures. Moving a `Vec` through
`std::sync::mpsc` does not copy its sample allocation, so Burn's custom unsafe
task arena should not be copied directly. The applicable lesson is to measure
the complete handoff, then eliminate avoidable allocation/copying and batch
coordination where doing so does not compromise playback latency or
backpressure.

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/playback/engine/mix/output_stage.rs` | Every output slice calls `chunk.to_vec()` before sending a `SamplesBuffer` |
| `proteus-lib/src/playback/engine/mix/runner/mod.rs` | Creates the bounded mix-to-worker `sync_channel(1)` |
| `proteus-lib/src/playback/engine/mix/runner/decode/mod.rs` | Allocates interleaved PCM vectors and sends owned packet events |
| `proteus-lib/src/playback/engine/mix/runner/startup.rs` | Creates the bounded multi-producer decode-event channel |
| `proteus-lib/src/playback/engine/mix/runner/loop_body.rs` | Polls and drains decode events as a consumer-side batch |
| `proteus-lib/src/playback/engine/mix/buffer_mixer/aligned_buffer.rs` | Copies routed slices into new vectors and currently shrinks owned buffers |
| `proteus-lib/src/playback/engine/mix/buffer_mixer/mixing.rs` | Allocates logical-track and final mix buffers per mix chunk |
| `proteus-lib/src/playback/engine/state.rs` | Existing DSP metrics can be extended with handoff/allocation diagnostics |
| `proteus-lib/src/diagnostics/` | Natural home for reproducible handoff benchmarks |

## Current state

Proteus uses two important bounded channel paths:

1. Decode workers send `DecodeWorkerEvent` values through a
   `sync_channel(64)` to the mix thread. Packet events own an interleaved
   `Vec<f32>`, and the mix loop already drains all currently available events
   with `try_recv()`.
2. The mix thread wraps output in `rodio::SamplesBuffer` and sends it through a
   `sync_channel(1)` to the playback worker. Output slicing deliberately uses
   the channel as per-slice backpressure, but each slice is first copied with
   `chunk.to_vec()`.

The channel envelopes are small; the large allocations are the PCM buffers
they point to. Likely costs include:

- per-packet channel interleaving allocations
- copies from decoded events into aligned sample-buffer segments
- `shrink_to_fit()` on owned routed buffers, which prevents useful capacity
  reuse and may itself reallocate
- per-chunk logical-track and final mix allocations
- per-output-slice `chunk.to_vec()` allocation/copying
- wakeup and synchronization overhead at very small output-slice sizes

There is currently no benchmark that separates channel coordination cost from
PCM allocation/copy cost, so replacing `std::sync::mpsc` would be speculative.

## Proposed work

### 1. Establish a representative baseline

Add benchmarks for both channel micro-cost and end-to-end playback handoffs.
Cover at least:

- one and multiple decode producers
- mono and stereo packets
- common packet/chunk sizes and the smallest supported live-authoring slices
- decode event enqueue/dequeue throughput
- mix-to-worker send/receive throughput
- total allocations and bytes allocated per second of audio
- p50/p95/p99 send or stall time
- CPU time, underruns/late appends, and queued sink latency in an integrated run

Benchmark release builds and report results with metering/debug logging both
disabled and enabled where relevant. A synthetic benchmark must be paired with
an actual decode/mix/playback workload, following the article's validation
approach.

### 2. Remove avoidable copies before changing channel implementations

Prototype safe ownership-preserving changes first:

- let unsliced output transfer an owned `Vec<f32>` directly into
  `SamplesBuffer` instead of accepting a slice and cloning it
- for sliced output, evaluate splitting/moving reusable owned buffers or a
  small pool of fixed-capacity chunk buffers
- allow the aligned buffer to consume owned packet storage directly when the
  full packet maps to one segment
- remove unconditional `shrink_to_fit()` from hot-path ownership transfers
- reuse interleave, logical-track, final-mix, and output buffers across chunks
  with explicit capacity bounds

Any pool must have a bounded fallback policy and return storage from the
consumer to the producer without allowing unbounded growth.

### 3. Evaluate batching without weakening latency bounds

The decode consumer already drains available events in a loop. Measure whether
an explicit batch message or producer-side batch swap reduces synchronization
under multi-source decode workloads.

Do not blindly apply Burn's double-buffer swap to the mix-to-output path. That
path uses `sync_channel(1)` to enforce backpressure, and batching can increase
audible control latency. Any batching proposal must preserve:

- packet/event ordering
- decode backpressure accounting
- bounded memory
- shutdown/disconnection behavior
- `max_sink_latency_ms`, `max_sink_chunks`, and `output_slice_ms` semantics

### 4. Compare safe channel alternatives only after allocation cleanup

Once payload allocation is controlled, benchmark the current
`std::sync::mpsc::sync_channel` against suitable bounded alternatives, such as
`crossbeam-channel` or a narrowly scoped SPSC queue for the single-producer
mix-to-worker path.

Adopt an alternative only if the integrated benchmark shows a meaningful,
repeatable improvement. A custom unsafe arena/channel requires a separate
design review, loom/Miri-oriented validation where applicable, documented
Acquire/Release contracts, and evidence that safe alternatives are
insufficient.

### 5. Add diagnostics and regressions

Expose enough debug metrics to detect regressions, for example:

- handoff messages and PCM bytes per second
- output/decode buffer allocation or pool-miss counts
- channel-full/block duration
- maximum decode batch size
- late append/underrun correlation with handoff stalls

Add tests for ordering, disconnect shutdown, abort while blocked, seek/reset,
end-of-stream drain, pool capacity bounds, and exact sample preservation.

## Non-goals

- Copying Burn's closure-specific inline task representation.
- Introducing TLS recursion handling; Proteus's audio handoffs do not have
  Burn's recursive task-submission requirement.
- Replacing channels based on microbenchmarks alone.
- Increasing queue depth or batch size at the expense of audible latency.
- Introducing unsafe code before a measured safe implementation proves
  inadequate.

## Acceptance criteria

- [ ] Baselines distinguish channel overhead from PCM allocation/copy overhead.
- [ ] Benchmarks include multi-producer decode and bounded mix-to-worker paths.
- [ ] At least the unsliced output path transfers sample ownership without a
      redundant `to_vec()` copy.
- [ ] Reusable buffers or another bounded allocation strategy materially reduce
      steady-state allocations in a representative playback workload.
- [ ] Ordering, backpressure, shutdown, seek, and end-of-stream behavior remain
      covered by tests.
- [ ] Live-authoring latency and late-append metrics do not regress.
- [ ] Any channel replacement is justified by integrated release-mode results,
      not only an isolated throughput benchmark.
- [ ] Before/after benchmark data and the chosen memory-ordering/ownership
      contract are documented.

## Status

Not started. The article identifies a credible optimization direction, but the
highest-confidence first step for Proteus is measurement plus safe buffer
ownership/reuse, not a custom unsafe channel.
