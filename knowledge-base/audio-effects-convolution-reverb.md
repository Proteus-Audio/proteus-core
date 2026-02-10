# Audio Effect: Convolution Reverb

## What it is
A **convolution reverb** that uses an impulse response (IR) to reproduce the acoustics of a real space or device. It performs FFT‑based convolution and mixes dry/wet audio.

## How it behaves (plain language)
- The input signal is “multiplied” by an impulse response in the frequency domain.
- This recreates the sound of the captured space (church, room, plate, etc.).
- The result can be highly realistic but more CPU‑intensive.

## How it works (step‑by‑step)
1. Resolve the impulse response (IR) spec from settings or the container context, and trim the tail using the configured `impulse_response_tail_db` if provided.
2. Build a per‑channel convolution engine using a fixed FFT size (`8192`), one `Convolver` per output channel.
3. Buffer incoming interleaved samples until a full convolution batch is available (`block_size * REVERB_BATCH_BLOCKS`).
4. De‑interleave the batch into per‑channel frames, then for each channel:
5. Split the frame into half‑FFT segments, FFT each segment, and push it into the overlap‑add history.
6. Multiply each FFT segment by the pre‑FFT’d IR segments, sum all segment products, then IFFT to time‑domain.
7. Add the saved overlap tail to the first half‑segment, save the new tail, and queue any excess output.
8. Re‑interleave channels and mix dry/wet (`dry_wet`) per sample.
9. If draining, flush any buffered output that remains in the overlap‑add pipeline.

## Signal Flow (simplified)

```
Input ─► FFT ─► * IR Spectrum ─► IFFT ─► Wet ──┐
   └──────────────────────────────────────────► Dry ├─► Mix ─► Output
```

## Key controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `dry_wet` | Dry/wet mix | More/less reverb |
| `enabled` | Bypass when false | Dry only |
| `impulse_response_*` | Which IR to load | Changes the “space” |
| `impulse_response_tail_db` | Tail trimming threshold | Shorter/longer tail |

## Performance characteristics

| Property | Value |
| --- | --- |
| Latency | Medium (block‑based) |
| CPU cost | Medium‑to‑high |
| Realism | High |

## Why block size matters (visual)

Convolution is block‑based. Output is continuous only when chunks align with the preferred batch size.

```
Misaligned chunks:
Input:  [chunk][chunk][chunk]
Conv:   [batch-----][batch-----]
Output: [ok][zeros][ok]  -> discontinuity risk

Aligned chunks:
Input:  [batch-----][batch-----]
Conv:   [batch-----][batch-----]
Output: [ok][ok]         -> smooth
```

## Practical note
If the mixer chunk size doesn’t align to the convolution batch size, you can get boundary discontinuities. The fix is to align chunk sizes to the preferred batch size (see `convolution-reverb-boundary-discontinuity.md`).
