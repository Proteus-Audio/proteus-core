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

## Proteus Context

Proteus's [`DiffusionReverb`](../audio-effects/diffusion-reverb.md) uses a Schroeder/Moorer-inspired structure with
additional diffusion and tonal smoothing:

- input pre-delay
- input all-pass diffusion stages
- parallel low-pass-feedback comb tank
- output all-pass diffusion stages
- light wet-output low-pass tone shaping

Proteus also runs one decorrelated reverb lane per channel (small channel-specific
delay offsets) to reduce metallic ringing and stereo correlation artifacts.

## Variable Key
- $`x[n]`$: input sample
- $`y_c[n]`$: comb output
- $`y_a[n]`$: all-pass output
- $`g`$: comb feedback gain, $`|g| < 1`$
- $`a`$: all-pass coefficient, $`|a| < 1`$
- $`D`$: comb delay (samples)
- $`M`$: all-pass delay (samples)

## Related

- [Algorithm: Comb Filter (Feedback)](./comb-filter.md)
- [Algorithm: All-Pass Filter (Delay Form)](./all-pass-filter.md)
- [Audio Effect: Diffusion Reverb](../audio-effects/diffusion-reverb.md)
- [Audio Effect: Delay Reverb](../audio-effects/delay-reverb.md)
