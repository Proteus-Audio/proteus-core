# Algorithm: Comb Filter (Feedback)

## Context and History
Comb filters are one of the oldest DSP building blocks and were used heavily in early reverb and flanger designs. In reverb, parallel feedback comb filters create dense resonant echo patterns and exponential decay.

## Mathematical Form

```text
y[n] = x[n] + g * y[n - D]
```

This is a feedback comb filter. A feedforward form also exists:

```text
y[n] = x[n] + b * x[n - D]
```

## Variable Key
- `x[n]`: input sample
- `y[n]`: output sample
- `g`: feedback gain
- `b`: feedforward gain
- `D`: delay in samples
