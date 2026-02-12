# Algorithm: Bilinear Transform (BLT)

## Context and History
The bilinear transform maps analog transfer functions to digital IIR filters and became a standard technique in digital filter design. It preserves stability and maps the analog left-half plane inside the unit circle.

## Mathematical Form
The substitution is:

```text
s = (2/T) * (1 - z^-1) / (1 + z^-1)
```

where `T = 1/fs`. Frequency prewarping is often used for accurate cutoff placement.

## Variable Key
- `s`: analog Laplace variable
- `z`: digital z-transform variable
- `T`: sample period
- `fs`: sample rate
