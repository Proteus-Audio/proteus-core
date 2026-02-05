## Other likely performance hotspots to investigate

Here’s a concrete list of places that can yield real gains:

### 1) Mixing strategy (biggest win)

Right now you:

- build a dynamic_mixer,
- convert buffers into SamplesBuffer,
- then collect into a Vec<f32>.

That allocates and copies every mix cycle.

Better: custom mix into a preallocated Vec<f32> (or ring buffer) and reuse it. That removes most allocations
and SamplesBuffer conversions.

———

### 2) Reverb worker IPC

You send a Vec<f32> over a sync channel, then allocate a new Vec<f32> in the worker. That’s extra copies and
synchronization.

Better:

- Use a reusable buffer pool (e.g. Vec<Vec<f32>>).
- Or move reverb into the same thread and avoid IPC if the worker isn’t needed.

———

### 3) Buffer map locking

buffer_map is a single Mutex<HashMap<...>>. The mixer loop holds it while reading and popping, and producers
also lock it to push.

That creates contention.

Better:

- Use per‑track locks (map of Arc<Mutex<Bounded<_>>>).
- Or use dashmap to reduce contention.

———

### 4) Non‑blocking container buffering

In container mode you now drop samples when buffers are full. That keeps things moving but can lead to
choppiness if sustained.

Better:

- Use the condvar to wait when full, but only block per‑track.
- Or add backpressure by pausing that specific track’s decoding.

———

### 5) Chunk size stability

The “smallest buffer” heuristic makes chunk size volatile.

Better:

- Target a fixed chunk size (e.g. 4096 frames).
- If any track doesn’t have enough, wait.
  This stabilizes performance and mixing overhead.

———

### 6) Effects buffer integration

Effects buffer and track buffers are mixed together, but effects buffer often is empty. The current code
still locks and checks it each pass.

Better:

- Only lock effects buffer when you know it’s non‑empty (a flag + notify).

———

## Recommendation order (highest impact first)

1. Fixed chunk size mixing with preallocated output buffer.
2. Per‑track buffer locks to reduce contention.
3. Reverb worker buffer reuse / avoid IPC.
4. Smarter container backpressure (avoid dropping).
