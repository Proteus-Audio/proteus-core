# Algorithm: Bilinear Transform (BLT)

## Context and History
The bilinear transform maps analog transfer functions to digital IIR filters and became a standard technique in digital filter design. It preserves stability and maps the analog left-half plane inside the unit circle.

## Mathematical Form
The substitution is:

```math
s = \frac{2}{T}\,\frac{1-z^{-1}}{1+z^{-1}}
```

with `$T = 1/f_s$`. Frequency prewarping is often used for accurate cutoff placement.

## Variable Key
- `$s$`: Laplace-domain complex variable
- `$z$`: z-domain complex variable
- `$T$`: sample period (seconds)
- `$f_s$`: sample rate (Hz)
