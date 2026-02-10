# Audio Effect: Limiter

## What it is
A **limiter** is a compressor with a very high ratio (often ∞:1). It prevents the signal from exceeding a ceiling.

## How it behaves (plain language)
- Anything above the threshold is clamped or heavily reduced.
- It protects against clipping and can raise overall loudness.

## Gain Curve (visual)

```
Output
1.0 ┤         ________ (ceiling)
    │        /
    │       /
    │      /
    │     /
0.0 ┼────/──────────── Input
      threshold
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `threshold` | Max level allowed | Lower = more limiting |
| `release` | How quickly it recovers | Short = tighter, long = smoother |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Prevent digital clipping
- Increase perceived loudness safely

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | Transparent if set gently |
