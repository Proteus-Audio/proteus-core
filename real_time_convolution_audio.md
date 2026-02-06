# Smooth Real-Time Audio with Convolution Reverb

This document summarizes practical strategies to prevent choppy playback when adding convolution reverb to a real-time Rust audio pipeline.

---

## 1. Architecture (Hard Requirement)

**Separate responsibilities clearly:**

- **Playback / audio callback**
  - Reads from ring buffer only
  - No allocation, no locks, no blocking
  - Must always run in bounded time

- **DSP worker thread**
  - Performs convolution and all heavy processing
  - Writes processed blocks into ring buffer

---

## 2. Buffering Strategy (Low Latency + Stability)

Use a *watermark-based ring buffer policy*.

### Recommended thresholds
- **Start threshold:** 10–30 ms  
  (wait to start playback until this much audio is buffered)
- **Low watermark:** 5–15 ms  
  (below this = danger zone)
- **High watermark:** 30–100 ms  
  (cap to avoid runaway latency)

This acts as a small jitter buffer to absorb CPU spikes while still starting nearly immediately.

---

## 3. Convolution: Use a Real-Time Friendly Algorithm

### Partitioned FFT Convolution (Best Practice)

- Split impulse response (IR) into partitions
- Use:
  - **Small early partition** (64–256 samples) → low latency
  - **Larger later partitions** (1024–8192 samples) → efficiency

Implementation notes:
- Precompute FFTs of IR partitions
- Reuse FFT plans and buffers
- Avoid allocations in the audio path

Avoid single large FFTs per block — they cause latency and compute spikes.

---

## 4. Underrun Protection (Never Sound Broken)

When buffer drops below a **panic threshold** (~2–5 ms):

Choose one:
1. Temporarily reduce or bypass reverb (fade wet → dry)
2. Switch to a cheaper reverb mode (shorter IR, fewer partitions)
3. Fade to silence (last resort, but better than stutter)

Crossfade transitions to avoid clicks.

---

## 5. Scheduling & Performance Hygiene

- Use lock-free ring buffers
- Pin DSP thread to a core if possible
- Raise thread priority carefully
- Avoid inconsistent block sizes
- Prevent denormals (flush-to-zero or tiny DC offset)

---

## Suggested Default Setup

- Start playback at **~20 ms buffered**
- Target steady-state buffer: **30–60 ms**
- Panic threshold: **~5 ms**
- Partitioned convolution with small early partition

This yields smooth playback with near-instant start and robust behavior under load.
