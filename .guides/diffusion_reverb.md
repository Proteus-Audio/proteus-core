# Diffusion Reverb (Algorithmic) Notes

This guide documents the current `DiffusionReverb` implementation in
`proteus-lib/src/dsp/effects/diffusion_reverb/mod.rs`.

## Current Topology (2026-02-24)

The implementation is a Schroeder-inspired algorithmic reverb with extra
diffusion and per-channel decorrelation:

1. Pre-delay
2. Input diffusion (`3` series allpass filters)
3. Late reverb tank (`8` parallel lowpass-feedback comb filters)
4. Output diffusion (`3` series allpass filters)
5. Gentle wet-output lowpass tone shaping

Important implementation detail:
- Reverb is processed as one independent lane per channel (e.g. separate L/R
  lanes for stereo), with small channel-specific delay offsets to reduce
  correlation and metallic stereo ringing.

## Why It Sounds Less Metallic Than The Older Version

Compared to the earlier version (4 combs + 2 allpasses, shared interleaved
network), the current version improves tail quality by:

- Increasing modal density (8 combs instead of 4)
- Adding input diffusion before the comb bank (smears transients earlier)
- Adding output diffusion after the comb bank (smooths late tail)
- Separating channels into decorrelated lanes (prevents cross-channel coupling)
- Applying a light post-wet lowpass (reduces harsh high-frequency ringing)

## User-Facing Controls

The public settings remain:

- `pre_delay_ms`
- `room_size_ms`
- `decay`
- `damping`
- `diffusion`
- effect `mix`

These are mapped internally into the denser topology; no schema change was
required for the update.

## Practical Tuning (Warmer / Deeper / Lusher)

Use these heuristics when dialing in the effect:

- Increase `room_size_ms` before pushing `decay`
  - Larger delay spacing usually reads as “deeper room” more naturally than
    simply increasing feedback.
- Raise `damping` to reduce metallic brightness
  - Typical warm range: `0.45..0.65`
  - Lower values sound brighter and more reflective.
- Keep `diffusion` moderately high, not maxed
  - Typical lush range: `0.65..0.80`
  - Too low sounds grainy; too high can blur attacks and produce a cloudy tail.
- Use `decay` conservatively
  - Typical musical range: `0.70..0.85`
  - Very high values can still emphasize resonant modes on tonal/percussive material.
- Set `mix` based on insert vs send usage
  - Insert: lower mix to preserve articulation
  - Send/aux: higher mix is usually appropriate

## Example Starting Points

### Warm Room / Plate-like

- `pre_delay_ms`: `8..16`
- `room_size_ms`: `40..60`
- `decay`: `0.72..0.80`
- `damping`: `0.48..0.62`
- `diffusion`: `0.68..0.78`

### Deep Ambient Tail

- `pre_delay_ms`: `16..28`
- `room_size_ms`: `55..85`
- `decay`: `0.80..0.88`
- `damping`: `0.50..0.70`
- `diffusion`: `0.72..0.82`

## Future Improvements (If Needed)

If the tail still sounds too static on exposed sources, the next high-value
upgrade is subtle delay modulation (usually on diffuser and/or comb lengths).
That is the standard step toward a more “lush” chorused algorithmic reverb.
