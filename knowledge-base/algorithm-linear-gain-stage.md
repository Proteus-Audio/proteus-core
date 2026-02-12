# Algorithm: Linear Gain Stage

## Context and History
A linear gain stage is the most fundamental operation in digital audio. It appears in mixers, trim controls, automation curves, and normalization pipelines.

## Mathematical Form

```text
y[n] = g * x[n]
```

For dB control values:

```text
g = 10^(dB/20)
```

## Variable Key
- `x[n]`: input sample
- `y[n]`: output sample
- `g`: linear gain factor
- `dB`: gain in decibels
