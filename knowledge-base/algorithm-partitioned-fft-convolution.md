# Algorithm: Partitioned FFT Convolution

## Context and History
Fast convolution became practical for audio after FFT methods were adopted for long FIR filtering. Partitioning the impulse response into blocks enabled real-time convolution reverb with long IRs.

## Mathematical Form
Time-domain convolution:

```text
y[n] = sum_{k=0}^{L-1} h[k] * x[n-k]
```

Partitioned frequency-domain form (block index `p`):

```text
Y_p(omega) = sum_{i=0}^{P-1} H_i(omega) * X_{p-i}(omega)
```

Then `y_p[n]` is recovered by IFFT per block.

## Variable Key
- `x[n]`: input signal
- `h[k]`: FIR impulse response
- `y[n]`: output signal
- `X_p(omega)`: FFT of input block `p`
- `H_i(omega)`: FFT of IR partition `i`
- `Y_p(omega)`: output spectrum for block `p`
- `L`: IR length
- `P`: number of partitions
