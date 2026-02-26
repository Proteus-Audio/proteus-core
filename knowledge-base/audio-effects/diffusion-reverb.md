# Audio Effect: Diffusion Reverb

## What it is
A **Schroeder-inspired algorithmic reverb** with extra diffusion stages, low-pass damped comb feedback, and per-channel decorrelated lanes for a smoother, less metallic tail.

## How it behaves (plain language)
- The input gets a short pre-delay, then is diffused before it reaches the reverb tank.
- Multiple comb filters build the decay tail while damping reduces high-frequency build-up.
- Additional output diffusion and a gentle wet low-pass soften the late tail.
- Stereo (or multi-channel) input is processed with separate decorrelated lanes, which reduces metallic stereo ringing.

## How it works (step‑by‑step)
1. Convert `pre_delay_ms` and `room_size_ms` into sample counts.
2. Build a tuning profile per channel lane: 1 pre-delay line, 3 input all-pass diffusers, 8 parallel comb filters, and 3 output all-pass diffusers.
3. Apply small channel-specific delay offsets so each lane is decorrelated.
4. For each incoming sample, route each interleaved channel sample into its matching reverb lane.
5. Apply the lane pre-delay.
6. Pass the signal through 3 input all-pass diffusers (transient smearing / early density).
7. Feed the diffused sample into 8 comb filters in parallel.
8. Each comb filter uses low-pass damping in its feedback loop, then feeds back by `decay`.
9. Average the comb outputs and pass the result through 3 output all-pass diffusers.
10. Apply a gentle one-pole low-pass to the wet signal to soften high-frequency ringing.
11. Mix wet with dry using `mix` (`output = dry*(1‑mix) + wet*mix`).
12. If draining, feed zeros through the lanes long enough to emit the tail.

## Signal Flow (simplified)

```
Input ─► Pre-delay ─► Input Diffusion ─► Comb Tank ─► Output Diffusion ─► Wet Tone LPF ─► Wet
   └───────────────────────────────────────────────────────────────────────────────────────► Dry
                                                     └──────────── Mix ───────────────────► Output
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `dry_wet` / `wet_dry` (`mix`) | Blend between dry and wet | More/less reverb presence |
| `enabled` | Bypass when false | Dry only |
| `pre_delay_ms` | Gap before reverb starts | More clarity / perceived depth |
| `room_size_ms` | Internal delay scaling | Bigger/deeper space impression |
| `decay` | Comb feedback amount | Longer/shorter tail |
| `damping` | HF damping in feedback | Darker/warmer vs brighter/metallic |
| `diffusion` | Diffuser feedback | Smoother/denser vs grainier |

## Technical
The design is a **multi-stage comb + all-pass diffusion network**: input all-pass diffusion spreads transients before the comb tank, 8 parallel low-pass-feedback combs build the late decay, then output all-pass diffusion and a light wet low-pass smooth the tail.

The implementation is still in the Schroeder/Moorer family, but uses a denser topology than a minimal comb+all-pass network. In Proteus specifically, channel lanes are decorrelated (small delay-length offsets per channel) to avoid the metallic artifacts that can occur when interleaved multi-channel samples share one scalar delay network.

Its research precedent is that dense, decorrelated reflections can be synthesized without convolution by carefully tuned delay lengths, feedback, and damping. The low-pass damping in feedback loops follows the same practical model used in many algorithmic reverbs to emulate high-frequency air/surface absorption over time.

## Tuning for a warmer / deeper / lusher sound

- Increase `room_size_ms` before pushing `decay`
  - Bigger spacing often sounds deeper and less “ringy” than just increasing feedback.
- Raise `damping` for warmth (`~0.45..0.65`)
  - This is the main control for reducing metallic brightness.
- Keep `diffusion` moderately high (`~0.65..0.80`)
  - Higher density smooths the tail, but maxing it can blur attacks.
- Use `decay` conservatively (`~0.70..0.85`) for general musical use
  - Very high values can still emphasize resonances on tonal/percussive sources.
- Adjust `mix` by routing style
  - Insert: lower `mix`
  - Send/aux: higher `mix`

## Typical use
- Smoothing harsh transients
- Creating a warm room / ambient algorithmic reverb
- A lightweight alternative to convolution when CPU budget matters

## Key properties

| Property | Value |
| --- | --- |
| Latency | Low‑to‑medium |
| CPU cost | Low‑to‑medium (higher than simple delay reverb; much lower than convolution) |
| Tail character | Smooth, denser, darker, less metallic than the earlier sparse topology |

## Related

- [Algorithm: Schroeder-Moorer Algorithmic Reverb](../algorithm/schroeder-moorer-reverb.md)
- [Algorithm: Comb Filter (Feedback)](../algorithm/comb-filter.md)
- [Algorithm: All-Pass Filter (Delay Form)](../algorithm/all-pass-filter.md)
- [Audio Effect: Delay Reverb](./delay-reverb.md)
- [Audio Effect: Convolution Reverb](./convolution-reverb.md)
