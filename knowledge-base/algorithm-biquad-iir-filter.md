# Algorithm: Biquad IIR Filter

## Context and History
The second-order IIR "biquad" is a core digital filter structure used in equalizers, crossovers, and tone controls. It is efficient, stable with proper coefficients, and easy to retune in real time.

## Mathematical Form

```math
y[n] = b_0x[n] + b_1x[n-1] + b_2x[n-2] - a_1y[n-1] - a_2y[n-2]
```

Low-pass, high-pass, shelving, and peaking filters are produced by choosing different coefficient sets.

## Variable Key
- `$x[n]$`: input sample
- `$y[n]$`: output sample
- `$b_0,b_1,b_2$`: feedforward coefficients
- `$a_1,a_2$`: feedback coefficients
