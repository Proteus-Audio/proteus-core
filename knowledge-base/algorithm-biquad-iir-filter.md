# Algorithm: Biquad IIR Filter

## Context and History
The second-order IIR "biquad" is a core digital filter structure used in equalizers, crossovers, and tone controls. It is efficient, stable with proper coefficients, and easy to retune in real time.

## Mathematical Form

```text
y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
```

Low-pass, high-pass, shelving, and peaking filters are produced by choosing different coefficient formulas.

## Variable Key
- `x[n]`: input sample
- `y[n]`: output sample
- `b0,b1,b2`: feedforward coefficients
- `a1,a2`: feedback coefficients
