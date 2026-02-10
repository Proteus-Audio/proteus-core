# Audio Effect: Convolution Reverb

## What it is
A **convolution reverb** that uses an impulse response (IR) to reproduce the acoustics of a real space or device. It performs FFT‑based convolution and mixes dry/wet audio.

## How it behaves (plain language)
- The input signal is “multiplied” by an impulse response in the frequency domain.
- This recreates the sound of the captured space (church, room, plate, etc.).
- The result can be highly realistic but more CPU‑intensive.

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
