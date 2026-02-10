# Audio Effect: Limiter

## What it is
A **limiter** is a compressor with a very high ratio (often ∞:1). It prevents the signal from exceeding a ceiling.

## How it behaves (plain language)
- Anything above the threshold is clamped or heavily reduced.
- It protects against clipping and can raise overall loudness.

## How it works (step‑by‑step)
1. Sanitize settings (threshold ≤ 0 dB, knee width ≥ 0.1 dB, attack/release ≥ 0).
2. Wrap the incoming audio in a small queued source that can be pulled sample‑by‑sample.
3. Feed queued samples into `rodio::source::Limit`, which applies the limiter curve with the given threshold, knee width, attack, and release.
4. Pull the same number of samples back out and return them as the processed output.

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
