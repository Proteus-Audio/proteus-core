# Audio Effect: Compressor

## What it is
A **dynamic range compressor** that reduces loud peaks and evens out volume.

## How it behaves (plain language)
- When the signal exceeds a threshold, gain is reduced.
- Quiet parts stay untouched (unless makeup gain is applied).
- The result is smoother, more consistent loudness.

## How it works (step‑by‑step)
1. Sanitize settings: clamp `threshold_db` to ≤ 0 dB, `ratio` to ≥ 1, and convert `attack_ms`/`release_ms` into smoothing coefficients.
2. Process audio in frames (one frame = all channels for a single sample time).
3. For each frame, compute the peak absolute sample across channels.
4. Convert the peak to dB (`20*log10`).
5. Compute the target gain reduction in dB:
6. If below threshold, target gain is `0 dB` (no change).
7. If above threshold, compress to `threshold + (level‑threshold)/ratio`, then take the difference to get a negative gain value.
8. Smooth the gain toward the target using attack when gain is decreasing and release when it is recovering.
9. Convert the smoothed gain + `makeup_gain_db` back to linear, then multiply all samples in the frame by that gain.

## Gain Reduction Curve (visual)

| Input Level | Output Level |
| --- | --- |
| Below threshold | Same as input |
| Above threshold | Reduced by ratio |

```
Output
1.0 ┤          __
    │         /
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
| `threshold` | Level where compression starts | Lower = more compression |
| `ratio` | How much to reduce above threshold | Higher = stronger control |
| `attack` | How fast compression engages | Faster = tighter |
| `release` | How fast it lets go | Faster = more pumping |
| `makeup_gain` | Adds gain after compression | Louder output |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Control peaks on vocals or drums
- Increase perceived loudness
- Glue multiple tracks together

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | Depends on settings (can be transparent or punchy) |
