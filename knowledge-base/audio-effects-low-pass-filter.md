# Audio Effect: Low‑Pass Filter

## What it is
A **low‑pass filter** that removes high frequencies above a cutoff point.

## How it behaves (plain language)
- Frequencies below the cutoff stay.
- Frequencies above the cutoff are reduced.
- The sound becomes **darker / softer**.

## How it works (step‑by‑step)
1. Sanitize `cutoff_hz` (stored as `freq_hz` in settings) to stay below Nyquist and clamp `q` to a safe range.
2. Compute biquad coefficients for a low‑pass BLT filter from the sample rate, cutoff, and Q.
3. Maintain per‑channel filter state (`x[n‑1]`, `x[n‑2]`, `y[n‑1]`, `y[n‑2]`).
4. For each interleaved sample, apply the biquad difference equation:
5. `y[n] = b0*x[n] + b1*x[n‑1] + b2*x[n‑2] − a1*y[n‑1] − a2*y[n‑2]`
6. Update the per‑channel history and write `y[n]` to the output.

## Frequency View (visual)

| Frequency Range | Output |
| --- | --- |
| Low | Pass |
| Mid | Pass (partial) |
| High | Attenuate |

```
Gain
1.0 ┤███████████▌
    │          ╲
    │           ╲
0.0 ┼────────────╲──────── Frequency
         cutoff
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `cutoff_hz` | Cutoff frequency | Lower = darker |
| `enabled` | Bypass when false | Dry only |

## Technical
This filter is a **second-order IIR biquad low-pass** implemented with the standard difference equation (`Direct Form` state update). Coefficients are derived from an analog prototype transformed to digital via the **bilinear transform (BLT)**.

The practical coefficient model is the same family used in Robert Bristow-Johnson's well-known "Audio EQ Cookbook" formulas (cutoff + Q mapped to stable biquad coefficients). This is the established baseline for efficient real-time tone-shaping filters in music/audio engines.

## Typical use
- Remove hiss or harshness
- Create warm, muffled tones
