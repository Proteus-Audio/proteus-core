# Audio Effect: Gain

## What it is
A **linear gain** stage that multiplies every sample by a constant factor.

## How it behaves (plain language)
- Makes the signal louder or quieter.
- Does not change tone, just level.
- Can be used for simple boosts or trims.

## How it works (step-by-step)
1. If `enabled` is false, return the input samples unchanged.
2. Sanitize `gain` (non-finite values fall back to a safe default).
3. Multiply each sample by `gain` and return the result.

## JSON controls

| Field | Type | Meaning |
| --- | --- | --- |
| `enabled` | bool | Bypass when false |
| `gain` | number or string | Linear gain or dB string (e.g. `1.25`, `"6db"`, `"-2db"`) |

### dB strings
When `gain` is a string ending with `db`, the value is converted to linear gain using `10^(db/20)`.

## Typical use
- Trim levels between tracks
- Boost into a compressor or limiter
- Quick overall volume tweak

## Key properties

| Property | Value |
| --- | --- |
| CPU cost | Low |
| Latency | None |
| Tone | Neutral |
