# Audio Effect: Distortion

## What it is
A **waveshaping** effect that adds harmonic content by clipping or bending the waveform.

## How it behaves (plain language)
- Loud parts are flattened or curved.
- This creates new harmonics (grit, edge).
- The signal can be softened (soft clip) or harsh (hard clip).

## How it works (step‑by‑step)
1. Sanitize `gain` and `threshold` (non‑finite values fall back to defaults; threshold is treated as absolute).
2. For each sample, multiply by `gain`.
3. Clamp the result to `[-threshold, +threshold]` (hard clip).
4. Return the clipped samples unchanged in length.

## Waveform View (visual)

```
Clean:      Distorted (clipped):

   /\          ____
  /  \        /    \
 /    \      /      \
/      \____/        \____
```

## JSON controls

| Field | Type | Meaning |
| --- | --- | --- |
| `enabled` | bool | Bypass when false |
| `gain` | number or string | Linear gain or dB string (e.g. `2.0`, `"6db"`) |
| `threshold` | number or string | Linear clamp level or dB string (e.g. `0.5`, `"-6db"`) |

## Technical
The current algorithm is **hard clipping waveshaping**: pre-gain the sample, then clip to a symmetric threshold. In nonlinear-systems terms, this is a static nonlinearity with no memory, which introduces odd/even harmonics based on waveform symmetry and drive.

This has long precedent in digital audio distortion units as the simplest "drive + clip" architecture. It is intentionally minimal and deterministic compared with more complex analog-model methods (tube/transistor circuit simulation, dynamic waveshapers, oversampled anti-alias designs), prioritizing low CPU and predictable output limits.

## Typical use
- Add warmth or aggression
- Make quiet details more audible

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | From mild warmth to heavy crunch |
