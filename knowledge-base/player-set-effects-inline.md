# `Player::set_effects_inline`

`set_effects_inline` updates the active DSP chain without restarting playback.

## What it does

- Replaces `Player`'s shared effects vector with the provided chain.
- Keeps the current sink and playback thread running.
- Does not clear effect state or tails (for example reverb decay already in flight).
- Applies new settings to future chunks processed by the mix thread.
- Applies a short internal crossfade (default 25 ms) between old-chain and new-chain outputs to reduce inline edit clicks.
- Transition duration is configurable through `Player::set_inline_effects_transition_ms`.

## What it does not do

- No seek/restart.
- No explicit effect-state reset.
- No sink clear.

## When to use it

Use this for live authoring or UI edits where you want fewer audible artifacts while adjusting effects.

## Related API

- `set_effects`: legacy disruptive update path. It resets effect state and seeks to current time to apply immediately.
