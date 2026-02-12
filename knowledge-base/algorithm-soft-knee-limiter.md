# Algorithm: Soft-Knee Limiter

## Context and History
Limiters are high-ratio dynamics processors used to prevent overs and clipping. Soft-knee variants became common in mastering/broadcast because they reduce abrupt gain transitions around threshold.

## Mathematical Form
A common soft-knee static curve around threshold $`T`$ with knee width $`K`$:

```math
G(L) =
\begin{cases}
0, & L \le T - \frac{K}{2} \\
T - L, & L \ge T + \frac{K}{2} \\
-\dfrac{\left(L - T + \frac{K}{2}\right)^2}{2K}, & \text{otherwise}
\end{cases}
```

Apply smoothed gain in linear amplitude:

```math
y[n] = x[n]\cdot 10^{\frac{G[n]}{20}}
```

## Variable Key
- $`L`$: input level (dB)
- $`T`$: threshold (dB)
- $`K`$: knee width (dB)
- $`G`$: gain reduction (dB)
- $`x[n]`$: input sample
- $`y[n]`$: output sample
