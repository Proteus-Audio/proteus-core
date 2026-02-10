# Audio Effect: High‑Pass Filter

## What it is
A **high‑pass filter** that removes low frequencies below a cutoff point.

## How it behaves (plain language)
- Frequencies above the cutoff stay.
- Frequencies below the cutoff are reduced.
- The sound becomes **thinner / cleaner**.

## How it works (step‑by‑step)
1. Sanitize `cutoff_hz` (stored as `freq_hz` in settings) to stay below Nyquist and clamp `q` to a safe range.
2. Compute biquad coefficients for a high‑pass BLT filter from the sample rate, cutoff, and Q.
3. Maintain per‑channel filter state (`x[n‑1]`, `x[n‑2]`, `y[n‑1]`, `y[n‑2]`).
4. For each interleaved sample, apply the biquad difference equation:
5. `y[n] = b0*x[n] + b1*x[n‑1] + b2*x[n‑2] − a1*y[n‑1] − a2*y[n‑2]`
6. Update the per‑channel history and write `y[n]` to the output.

## Frequency View (visual)

| Frequency Range | Output |
| --- | --- |
| Low | Attenuate |
| Mid | Pass (partial) |
| High | Pass |

```
Gain
1.0 ┤          ███████████
    │        ╱
    │      ╱
0.0 ┼────╱───────────────── Frequency
     cutoff
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `cutoff_hz` | Cutoff frequency | Higher = thinner |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Remove rumble or mic handling noise
- Clean up low‑end buildup
