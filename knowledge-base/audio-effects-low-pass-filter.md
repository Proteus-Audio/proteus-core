# Audio Effect: Low‑Pass Filter

## What it is
A **low‑pass filter** that removes high frequencies above a cutoff point.

## How it behaves (plain language)
- Frequencies below the cutoff stay.
- Frequencies above the cutoff are reduced.
- The sound becomes **darker / softer**.

## Frequency View (visual)

| Frequency Range | Output |
| --- | --- |
| Low | Pass |
| Mid | Pass (partial) |
| High | Attenuate |

```
Gain
1.0 ┤███████████▌
    │          ╲
    │           ╲
0.0 ┼────────────╲──────── Frequency
         cutoff
```

## Controls (conceptual)

| Control | What it changes | Audible effect |
| --- | --- | --- |
| `cutoff_hz` | Cutoff frequency | Lower = darker |
| `enabled` | Bypass when false | Dry only |

## Typical use
- Remove hiss or harshness
- Create warm, muffled tones
