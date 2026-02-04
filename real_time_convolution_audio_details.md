What you’re seeing is the classic “producer can’t keep up with the consumer” problem. Your audio callback (or playback thread) is consuming samples at a fixed real-time rate, and your convolution step occasionally takes long enough that the ring buffer hits zero → underrun → choppy output.

The “best” solution is usually **a combination of (1) real-time-safe architecture, (2) a latency/buffering strategy with watermarks, and (3) a more efficient convolution algorithm** so your average compute stays comfortably faster than real time.

## 1) Separate real-time playback from heavy processing (non-negotiable)

If any convolution work is happening on the same thread that must deliver audio on time (often the audio callback), you’ll always be fighting glitches.

**Goal:** the playback side should *only*:

* pop already-ready frames from a lock-free structure
* do tiny, bounded work (gain, mixing, maybe a cheap limiter)
* never allocate, never lock, never block

**All heavy DSP (convolution) belongs on a worker thread** that pushes processed blocks into the ring buffer.

If you’re already doing this, good — then the rest is about keeping the buffer fed and lowering compute spikes.

## 2) Use a watermark / prefill strategy (smooth playback + minimal start delay)

You want playback smooth *and* start quickly. The clean way to balance those is:

### A. Pre-roll to a target fill level (tiny but intentional latency)

Instead of starting playback the moment you have *any* samples, start when the ring buffer reaches a **start threshold**:

* `start_threshold`: e.g. **10–30 ms** of audio buffered
* `low_watermark`: e.g. **5–15 ms**
* `high_watermark`: e.g. **30–100 ms** (cap to avoid runaway latency)

This gives you a tiny “jitter buffer” to absorb compute variance.

**What this achieves**

* Starts almost immediately (10–30 ms is usually perceived as instant in CLI playback)
* Prevents underruns when convolution occasionally spikes

### B. Keep it filled during steady state

Once playing, aim to keep buffered audio above `low_watermark`.

If buffer level drops below low watermark, you have choices:

* **Quality-preserving:** temporarily increase processing aggressiveness (bigger blocks, more CPU)
* **Glitch-preventing fallback:** reduce effect cost temporarily (see section 4)

### C. Measure it (don’t guess)

Track:

* buffer fill level over time
* underrun count
* processing time per block (mean + worst-case)

You’ll quickly see whether you need 10 ms or 50 ms of safety.

## 3) Make convolution real-time friendly (this is usually the real fix)

Time-domain convolution is *expensive*. Even FFT convolution can be expensive if done naïvely. For reverb IRs (often tens/hundreds of ms), the standard approach is:

### Partitioned convolution (Overlap-Save / Overlap-Add with partitions)

Split the impulse response into partitions and convolve in the frequency domain:

* Use a **small first partition** for low latency (e.g. 64–256 samples)
* Use **larger later partitions** for efficiency (e.g. 1024–8192 samples)

This is the typical “best of both worlds” structure:

* low perceived latency for early reflections
* efficient long tail

If you do a single huge FFT for the whole IR, your per-block compute spikes and latency get worse.

### Practical notes

* Precompute FFTs of IR partitions once.
* Reuse FFT plans (no per-block planning).
* Avoid allocations inside the processing loop.
* Prefer fixed-size buffers and reuse scratch space.

This one change often turns “sometimes too slow” into “comfortably real-time”.

## 4) Add graceful underrun protection (so it *never* sounds choppy)

Even with the above, your machine can get preempted, IR can change, CPU can spike, etc. A robust audio engine has a fallback when buffer gets dangerously low.

When buffer < “panic threshold” (e.g. 2–5 ms), do one of:

1. **Temporarily bypass or reduce the reverb** (output dry or reduce wet mix), then crossfade back in once buffer recovers.
2. **Switch to a cheaper reverb mode** (shorter IR / lower partition count) until stable.
3. **Repeat last sample / output silence** (worst sounding, but better than stuttery partial buffers). Prefer a short fade to zero to avoid clicks.

This makes failure modes sound intentional instead of broken.

## 5) Reduce scheduling jitter and CPU spikes

Once the algorithm is right and buffering is reasonable, the remaining glitches are often OS scheduling / contention:

* Put the DSP thread on a **dedicated core** (CPU affinity) if you can.
* Raise priority (on Linux use `SCHED_FIFO`/`SCHED_RR` carefully; on Windows “Pro Audio” / MMCSS; etc.).
* Avoid locks between threads; use lock-free ring buffers.
* Ensure processing block sizes are consistent; avoid “sometimes huge blocks”.
* Avoid denormals (add tiny DC offset or enable flush-to-zero if applicable).

## What I would implement (a solid default)

1. **Playback thread/callback**: only reads from ring buffer.
2. **DSP worker**: reads input blocks, runs partitioned convolution, writes output blocks.
3. **Buffer policy**:

   * start playback when buffer ≥ **20 ms**
   * target steady fill: **30–60 ms**
   * panic threshold: **5 ms** → reduce/bypass wet with a short crossfade
4. **Convolution**: partitioned FFT (small early partition, larger later partitions).

This gives you “starts basically immediately” while staying glitch-free under realistic CPU jitter.
