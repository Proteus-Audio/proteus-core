# Audio Effect: Distortion

## What it is
A **waveshaping** effect that adds harmonic content by clipping or bending the waveform.

## How it behaves (plain language)
- Loud parts are flattened or curved.
- This creates new harmonics (grit, edge).
- The signal can be softened (soft clip) or harsh (hard clip).

## Waveform View (visual)

```
Clean:      Distorted (clipped):

   /\          ____
  /  \        /    \
 /    \      /      \
/      \____/        \____
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `drive` | Input gain into shaper | More drive = more grit |
| `mix` | Dry/wet blend | Subtle to aggressive |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Add warmth or aggression
- Make quiet details more audible

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | From mild warmth to heavy crunch |
