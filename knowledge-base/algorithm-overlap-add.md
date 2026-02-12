# Algorithm: Overlap-Add (OLA)

## Context and History
Overlap-add is a standard block-processing reconstruction method used with FFT convolution and STFT-based systems. It resolves boundary overlap by summing adjacent block tails/heads.

## Mathematical Form
If block processing yields partial outputs `v_p[n]`, the full signal is reconstructed as:

```text
y[n] = sum_p v_p[n - pR]
```

where `R` is hop size.

## Variable Key
- `v_p[n]`: processed output block `p` in local block coordinates
- `y[n]`: reconstructed output signal
- `p`: block index
- `R`: hop size (samples)
