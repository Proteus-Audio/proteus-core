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
3. Buffer incoming interleaved samples in the internal state (`input_buffer`) and process in preferred batches (`block_size * REVERB_BATCH_BLOCKS`) when available.
4. De‑interleave the batch into per‑channel frames, then for each channel:
5. Split the frame into half‑FFT segments, FFT each segment, and push it into the overlap‑add history.
6. Multiply each FFT segment by the pre‑FFT’d IR segments, sum all segment products, then IFFT to time‑domain.
7. Add the saved overlap tail to the first half‑segment, save the new tail, and queue any excess output.
8. Re‑interleave channels and mix dry/wet (`dry_wet`) per sample.
9. If draining, flush any buffered output that remains in the overlap‑add pipeline.
10. If a chunk still underfills output length, fall back to dry input for the missing tail to avoid silence gaps.

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

## Technical
This effect uses **partitioned FFT convolution** with an **overlap-add** style reconstruction path. Converting long FIR convolution to frequency-domain block multiplication reduces complexity from direct O(N*M) time-domain convolution to a practical block-FFT pipeline suitable for real-time use.

The method follows established DSP precedent for long impulse responses in audio (fast convolution literature and production reverb engines): pre-FFT the IR partitions, FFT incoming partitions, multiply/accumulate in frequency domain, IFFT back, then manage overlaps/tails between blocks. This is the standard approach for realistic space emulation at manageable CPU cost.

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
Output: [ok][fallback][ok]  -> discontinuity risk

Aligned chunks:
Input:  [batch-----][batch-----]
Conv:   [batch-----][batch-----]
Output: [ok][ok]         -> smooth
```

## Practical note
If the mixer chunk size doesn't align to the convolution batch size, you can get boundary discontinuities. The fix is to align chunk sizes to the preferred batch size (see [Boundary Discontinuity Note](../convolution-reverb/boundary-discontinuity.md)).

## Related

- [Algorithm: Partitioned FFT Convolution](../algorithm/partitioned-fft-convolution.md)
- [Algorithm: Overlap-Add (OLA)](../algorithm/overlap-add.md)
- [Convolution Reverb Boundary Discontinuity](../convolution-reverb/boundary-discontinuity.md)
- [Player: `run_playback_thread` Sample Processing Flow](../player/run-playback-thread-sample-flow.md)
- [Audio Effect: Diffusion Reverb](./diffusion-reverb.md)
