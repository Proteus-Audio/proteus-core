# Audio Effect: Diffusion Reverb

## What it is
A **diffusion‑based reverb** that smears transients using short, dense delay networks to create a smooth, early‑reflection style reverb.

## How it behaves (plain language)
- The input passes through a network of short delays and all‑pass filters.
- This spreads the energy in time, reducing sharp transients.
- The result is a **cloudy, smooth** reverb tail without long echoes.

## How it works (step‑by‑step)
1. Convert `pre_delay_ms` and `room_size_ms` into sample counts.
2. Build a tuning profile: 1 pre‑delay line, 4 parallel comb filters, and 2 series all‑pass filters.
3. For each incoming sample:
4. Apply the pre‑delay line.
5. Feed the delayed sample into the 4 comb filters in parallel.
6. Each comb filter uses low‑pass damping in its feedback loop, then feeds back by `decay`.
7. Average the comb outputs and pass the result through two all‑pass filters in series.
8. Mix wet with dry using `mix` (`output = dry*(1‑mix) + wet*mix`).
9. If draining, feed zeros through the network long enough to emit the tail.

## Signal Flow (simplified)

```
Input ─► Diffusion Network (short delays / all‑pass) ─► Wet
   └───────────────────────────────────────────────► Dry
                          └────────── Mix ─────────► Output
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `dry_wet` | Blend between dry and wet | More/less diffusion |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Smoothing harsh transients
- Creating a “room‑like” or “ambience” style reverb

## Key properties

| Property | Value |
| --- | --- |
| Latency | Low‑to‑medium |
| CPU cost | Low‑to‑medium |
| Tail character | Smooth, short, dense |
