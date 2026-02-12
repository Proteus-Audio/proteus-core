# Algorithm: All-Pass Filter (Delay Form)

## Context and History
All-pass filters have flat magnitude response but frequency-dependent phase. In reverbs they are used for diffusion: they spread transients in time without large spectral coloring.

## Mathematical Form
One common delay-based all-pass form is:

```math
y[n] = -a\,x[n] + x[n-M] + a\,y[n-M]
```

Its ideal magnitude response is flat:

```math
\left|H\left(e^{j\omega}\right)\right| = 1
```

## Variable Key
- `$x[n]$`: input sample
- `$y[n]$`: output sample
- `$a$`: all-pass coefficient, `$|a| < 1$`
- `$M$`: delay (samples)
