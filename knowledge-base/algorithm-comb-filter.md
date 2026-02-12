# Algorithm: Comb Filter (Feedback)

## Context and History
Comb filters are one of the oldest DSP building blocks and were used heavily in early reverb and flanger designs. In reverb, parallel feedback comb filters create dense resonant echo patterns and exponential decay.

## Mathematical Form
Feedback comb filter:

```math
y[n] = x[n] + g\,y[n-D]
```

Feedforward comb filter:

```math
y[n] = x[n] + b\,x[n-D]
```

## Variable Key
- $`x[n]`$: input sample
- $`y[n]`$: output sample
- $`g`$: feedback gain
- $`b`$: feedforward gain
- $`D`$: delay (samples)
