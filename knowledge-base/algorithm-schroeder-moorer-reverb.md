# Algorithm: Schroeder-Moorer Algorithmic Reverb

## Context and History
Manfred Schroeder (early 1960s) introduced the classic digital artificial reverb structure based on comb and all-pass filters. James Moorer later extended this approach with practical refinements (early reflections, damping, tuning), and this became the basis for many low-cost algorithmic reverbs.

## Mathematical Form
A simple Schroeder-style core can be written as:

```math
y_c[n] = x[n] + g\,y_c[n-D]
```

```math
y_a[n] = -a\,x[n] + x[n-M] + a\,y_a[n-M]
```

Typical networks sum several comb filters in parallel and follow them with one or more all-pass stages in series.

## Variable Key
- `$x[n]$`: input sample
- `$y_c[n]$`: comb output
- `$y_a[n]$`: all-pass output
- `$g$`: comb feedback gain, `$|g| < 1$`
- `$a$`: all-pass coefficient, `$|a| < 1$`
- `$D$`: comb delay (samples)
- `$M$`: all-pass delay (samples)
