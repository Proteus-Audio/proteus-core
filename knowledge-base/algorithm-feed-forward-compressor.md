# Algorithm: Feed-Forward Peak Compressor

## Context and History
Feed-forward dynamics processors compute gain from the input signal rather than from output feedback. This became a dominant digital compressor architecture because it is predictable and easy to tune.

## Mathematical Form
Peak detector and static curve:

```math
L[n] = 20\log_{10}\!\left(\max_c\left|x_c[n]\right|\right)
```

```math
G_t[n] =
\begin{cases}
0, & L[n] \le T \\
\left(T + \frac{L[n]-T}{R}\right) - L[n], & L[n] > T
\end{cases}
```

Attack/release smoothing:

```math
G[n] = \alpha\,G[n-1] + (1-\alpha)\,G_t[n]
```

Gain application:

```math
y[n] = x[n] \cdot 10^{\frac{G[n]+M}{20}}
```

## Variable Key
- $`x_c[n]`$: input sample for channel $`c`$
- $`x[n]`$: input sample after channel/frame selection
- $`y[n]`$: output sample
- $`L[n]`$: detected level (dB)
- $`T`$: threshold (dB)
- $`R`$: ratio
- $`G_t[n]`$: target gain reduction (dB)
- $`G[n]`$: smoothed gain reduction (dB)
- $`M`$: makeup gain (dB)
- $`\alpha`$: smoothing coefficient (attack or release dependent)
