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

## Typical use
- Add warmth or aggression
- Make quiet details more audible

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | From mild warmth to heavy crunch |
