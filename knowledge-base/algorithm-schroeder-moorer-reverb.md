# Algorithm: Schroeder-Moorer Algorithmic Reverb

## Context and History
Manfred Schroeder (early 1960s) introduced the classic digital artificial reverb structure based on comb and all-pass filters. James Moorer later extended this approach with practical refinements (early reflections, damping, tuning), and this became the basis for many low-cost algorithmic reverbs.

## Mathematical Form
A simple Schroeder-style block can be written as:

```text
Comb:    y_c[n] = x[n] + g * y_c[n - D]
Allpass: y_a[n] = -a * x[n] + x[n - M] + a * y_a[n - M]
```

Typical networks sum several comb filters in parallel, then pass through one or more all-pass stages in series.

## Variable Key
- `x[n]`: input sample at time index `n`
- `y_c[n]`: comb filter output
- `y_a[n]`: all-pass filter output
- `g`: comb feedback gain (`|g| < 1` for stability)
- `a`: all-pass coefficient (`|a| < 1`)
- `D`: comb delay length (samples)
- `M`: all-pass delay length (samples)
