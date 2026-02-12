# Algorithm: Soft-Knee Limiter

## Context and History
Limiters are high-ratio dynamics processors used to prevent overs and clipping. Soft-knee variants became common in mastering/broadcast because they reduce abrupt gain transitions around threshold.

## Mathematical Form
A common soft-knee static curve is piecewise around threshold `Th` with knee width `K`:

```text
If L <= Th - K/2:        G = 0
If L >= Th + K/2:        G = Th - L
Else (knee region):      G = -((L - Th + K/2)^2) / (2K)
```

Then apply smoothed gain to signal:

```text
y[n] = x[n] * 10^(G[n]/20)
```

## Variable Key
- `L`: input level in dB
- `Th`: threshold in dB
- `K`: knee width in dB
- `G`: gain reduction in dB
- `x[n]`: input sample
- `y[n]`: output sample
