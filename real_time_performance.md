## Performance Re-Assessment (Feb 2026)

Observed:
- `OUT` ksps below the target sample rate even without reverb.
- With reverb enabled, `OUT` ksps drops further and audio becomes choppy.
- Buffer fill is steady, so the bottleneck is likely compute or per-chunk overhead rather than starvation.

This suggests the hot path is still the DSP/mix/reverb chain and/or per-chunk overhead, not I/O.

## Recommended Improvements (Ordered By Expected Impact)

### 1) Reduce Reverb Overhead (Biggest Win)
- Remove the reverb worker IPC copy by processing in the mix thread when enabled.
- Or keep the worker but use a shared buffer pool to avoid allocating/copying per chunk.
- Expected impact: large reduction in CPU and latency because the current IPC + allocations are on every chunk.

### 2) Fixed Chunk Size + Preallocated Mixing Buffer (Already Started)
- Keep fixed-size chunks (e.g., 10–50ms) and avoid variable-size chunking.
- Mix directly into a reusable `Vec<f32>` and avoid `dynamic_mixer` and repeated `SamplesBuffer` creation.
- Expected impact: reduces per-chunk overhead and stabilizes throughput.

### 3) Per-Track Buffer Locks (Reduce Contention)
- Replace the single `Mutex<HashMap<...>>` with per-track buffers (`Arc<Mutex<Bounded<_>>>` per track).
- Expected impact: lower lock contention between decoder threads and the mix loop.

### 4) Batch Reverb (Process Larger Blocks)
- Increase block size for reverb specifically, even if mix chunks are smaller.
- Example: accumulate N mix chunks, process reverb in a larger block, then output in smaller blocks.
- Expected impact: lower FFT overhead per second at the cost of slightly higher latency.

### 5) Smarter Container Backpressure (Avoid Dropping)
- In container mode, avoid dropping samples when buffers are full.
- Use the condvar to block only the specific track that is full.
- Expected impact: reduces audible glitches due to dropped samples.

### 6) Effects Buffer Integration (Minor)
- Avoid locking/reading the effects buffer when it is empty.
- Use a flag or notify to skip the lock path.

## Notes For Next Tests
- If `OUT` ksps is below the target sample rate, throughput is the primary issue.
- If `BUF` is stable but `OUT` is low, focus on CPU per chunk rather than buffering.


CHECK LIST:
- [ ] #1 - Reduce Reverb Overhead
- [x] #2 - Fixed Chunk Size + Preallocated Mixing Buffer (Already Started)
- [x] #3 - Per-Track Buffer Locks (Reduce Contention)
- [ ] #4 - Batch Reverb (Process Larger Blocks)
- [ ] #5 - Smarter Container Backpressure (Avoid Dropping)
- [ ] #6 - Effects Buffer Integration (Minor)
