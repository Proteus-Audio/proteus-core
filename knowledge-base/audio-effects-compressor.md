# Audio Effect: Compressor

## What it is
A **dynamic range compressor** that reduces loud peaks and evens out volume.

## How it behaves (plain language)
- When the signal exceeds a threshold, gain is reduced.
- Quiet parts stay untouched (unless makeup gain is applied).
- The result is smoother, more consistent loudness.

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
