# Audio Effect: Diffusion Reverb

## What it is
A **diffusion‑based reverb** that smears transients using short, dense delay networks to create a smooth, early‑reflection style reverb.

## How it behaves (plain language)
- The input passes through a network of short delays and all‑pass filters.
- This spreads the energy in time, reducing sharp transients.
- The result is a **cloudy, smooth** reverb tail without long echoes.

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
