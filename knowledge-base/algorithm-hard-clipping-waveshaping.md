# Algorithm: Hard Clipping Waveshaper

## Context and History
Hard clipping is one of the earliest and most common digital distortion methods. It models saturation crudely by limiting amplitude, producing strong harmonic content with very low compute cost.

## Mathematical Form
With pre-gain `g` and threshold `T`:

```text
u[n] = g * x[n]
y[n] = clamp(u[n], -T, T)
```

## Variable Key
- `x[n]`: input sample
- `u[n]`: driven sample before clipping
- `y[n]`: clipped output
- `g`: pre-gain (drive)
- `T`: clip threshold (`T > 0`)
