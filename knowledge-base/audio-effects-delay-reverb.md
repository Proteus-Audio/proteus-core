# Audio Effect: Delay Reverb

## What it is
A classic **delay‑based reverb** that feeds the signal through short delays with feedback to create a spacious echo tail. This is the effect formerly labeled “Basic Reverb” in the codebase and is still available via the deprecated `BasicReverb` alias.

## How it behaves (plain language)
- The input is copied into one or more delay lines.
- A portion of the delayed signal is fed back to create multiple echoes.
- The dry (original) and wet (delayed) signals are mixed together.

## Signal Flow (simplified)

```
Input ──┬─────────────► Dry ─────────────┐
        │                                ├─► Mix ─► Output
        └─► Delay(s) + Feedback ─► Wet ───┘
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `dry_wet` | Blend between dry and wet | More/less reverb presence |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Add subtle space to dry signals
- Thicken short percussive material

## Key properties

| Property | Value |
| --- | --- |
| Latency | Low (short delays) |
| CPU cost | Low |
| Tail length | Adjustable via feedback |

## Notes
- The public name is **DelayReverb**. The old **BasicReverb** name remains as a deprecated alias for compatibility.
