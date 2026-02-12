# Algorithm: All-Pass Filter (Delay Form)

## Context and History
All-pass filters have flat magnitude response but frequency-dependent phase. In reverbs they are used for diffusion: they spread transients in time without large spectral coloring.

## Mathematical Form
One common delay-based all-pass form is:

```text
y[n] = -a * x[n] + x[n - M] + a * y[n - M]
```

Its magnitude response is ideally unity while phase varies with frequency.

## Variable Key
- `x[n]`: input sample
- `y[n]`: output sample
- `a`: all-pass coefficient (`|a| < 1`)
- `M`: delay in samples
