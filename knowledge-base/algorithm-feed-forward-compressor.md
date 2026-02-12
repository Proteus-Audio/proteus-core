# Algorithm: Feed-Forward Peak Compressor

## Context and History
Feed-forward dynamics processors compute gain from the input signal rather than from output feedback. This became a dominant digital compressor architecture because it is predictable and easy to tune.

## Mathematical Form
Peak detection and static curve:

```text
L[n] = 20*log10(max(|x_ch[n]|))
G_target[n] = 0,                     if L[n] <= Th
G_target[n] = (Th + (L[n]-Th)/R) - L[n], otherwise
```

Ballistics smoothing (attack/release):

```text
G[n] = alpha * G[n-1] + (1-alpha) * G_target[n]
```

Output gain application:

```text
y[n] = x[n] * 10^((G[n] + M)/20)
```

## Variable Key
- `x_ch[n]`: channel sample at frame `n`
- `L[n]`: detected level in dB
- `Th`: threshold in dB
- `R`: ratio
- `G_target[n]`: target gain reduction in dB
- `G[n]`: smoothed gain reduction in dB
- `M`: makeup gain in dB
- `alpha`: smoothing coefficient (attack or release dependent)
