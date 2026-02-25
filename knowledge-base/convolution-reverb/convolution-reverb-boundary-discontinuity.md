# Convolution Reverb Boundary Discontinuity: Cause and Fix

This document explains (simply but thoroughly) why we saw audible boundary discontinuities when `ConvolutionReverb` was enabled, and how aligning the mix chunk size to the convolution **preferred batch size** fixes the issue without changing playback tempo.

## Problem Summary

- With `ConvolutionReverb` enabled, we saw **audible clicks/glitches** near the start of playback.
- Debug logs showed **boundary discontinuities** (large jumps between the last sample of one chunk and the first sample of the next).
- Other effects were smooth, so the discontinuity was introduced **inside the convolution reverb processing**.

## Key Concept: Overlap-Add Convolution Needs Aligned Blocks

The convolution reverb uses an **overlap-add FFT** algorithm. Internally it wants input in blocks aligned to the FFT segment size.

- FFT size = `8192`
- Segment size = `FFT_SIZE / 2 = 4096`
- For stereo (2 channels), **block size (interleaved samples)** = `4096 * 2 = 8192`
- The reverb processes **REVERB_BATCH_BLOCKS = 2** segments per batch
- So **preferred batch size** (interleaved samples) = `4096 * 2 * 2 = 16384`

When the mixer sends a chunk that is **not a multiple of this batch size**, the reverb buffers input until it can process a full batch. That can make effect output **lag** or **underfill** relative to chunk boundaries, which increases discontinuity risk at boundaries.

## Visual: What Was Happening

### Before (Misaligned Chunk Sizes)

Assume:
- Mixer chunk size = `300ms` (example) → **not** a multiple of preferred batch size
- Convolution wants `16384` interleaved samples per batch

Timeline (simplified):

```
Input chunks from mixer:   [C1........][C2........][C3........]
Convolver can process:     [----BATCH----][----BATCH----]
Output produced:           [OK][fallback][OK][fallback][OK]...
Boundary discontinuity:        ^^^^^^^ jump between fallback path and wet output
```

Historically, underfill could produce silence padding; current output-stage behavior falls back to dry input when an effect underfills. Either way, if chunk boundaries are misaligned with convolution batching, transitions can become discontinuous.

### After (Aligned to Preferred Batch Size)

Now the mixer only emits chunks that are multiples of the preferred batch size:

```
Input chunks from mixer:   [---BATCH---][---BATCH---][---BATCH---]
Convolver can process:     [---BATCH---][---BATCH---][---BATCH---]
Output produced:           [OK][OK][OK]...
Boundary discontinuity:    none
```

Because the reverb always receives a full batch, it can produce a full output without padding.

## The Fix: Preferred Batch Size Alignment

We compute and use a **preferred batch size** for convolution reverb:

| Value | Meaning | Example (stereo) |
| --- | --- | --- |
| `FFT_SIZE` | FFT length | `8192` |
| `segment_size` | `FFT_SIZE / 2` | `4096` |
| `block_samples` | `segment_size * channels` | `8192` |
| `REVERB_BATCH_BLOCKS` | batches per process | `2` |
| **preferred batch size** | `block_samples * REVERB_BATCH_BLOCKS` | **`16384`** |

Then, when `ConvolutionReverb` is enabled, the mixer chunk size is rounded **up** to the nearest multiple of the preferred batch size.

### Example Calculation

- Original mix chunk size: `14400` samples
- Preferred batch size: `16384` samples
- New mix chunk size: `ceil(14400 / 16384) * 16384 = 16384`

This ensures **every chunk** can be fully processed by the convolution engine without leftover underfill.

## Why It Also Fixed the Tempo Issue

An earlier attempt forced the reverb to process partial input immediately, which **changed the timing semantics** of the overlap-add pipeline. That caused the output to drift in time (audible BPM slowdown).

The preferred-batch approach **does not change the timeline**:
- It only changes chunk *boundaries*, not the timebase.
- The mixer still advances in real time, and each chunk covers the correct duration.

## Implementation Summary

- Expose a `preferred_batch_samples(channels)` helper in the convolution reverb module.
- When `ConvolutionReverb` is enabled, round `min_mix_samples` up to a multiple of this batch size.
- Leave convolution internals unchanged; no extra padding or partial processing.

## Quick Reference

| Symptom | Root Cause | Fix |
| --- | --- | --- |
| Boundary discontinuities | Output underfilled because chunk size not aligned to convolution batch size | Align mixer chunk size to preferred batch size |
| BPM slowdown | Forcing convolution to process partial input | Avoid partial processing; align chunk size instead |

## Takeaway

**Convolution reverb is block-based.** If the mixer feeds it chunks that don’t align to its batch size, the output can underfill, and padding creates discontinuities. Aligning the mixer’s chunk size to the **preferred batch size** makes the convolution output continuous and preserves correct timing.
