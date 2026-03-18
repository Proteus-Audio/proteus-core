# SI-12 — Deprecated backward-compatibility aliases in `dsp/effects`

## Rule

CLAUDE.md (Key Conventions): "Avoid backwards-compatibility hacks like …
re-exporting types."
Style Guide §11 (Code Smells): "Backwards-compatibility shims … if you are
certain that something is unused, delete it."

## Problem

`dsp/effects/basic_reverb/mod.rs` exports two deprecated type aliases:

```rust
#[deprecated(note = "Use DelayReverbSettings instead.")]
pub type BasicReverbSettings = DelayReverbSettings;

#[deprecated(note = "Use DelayReverbEffect instead.")]
pub type BasicReverbEffect = DelayReverbEffect;
```

`dsp/effects/mod.rs` then re-exports these and uses them in the `AudioEffect`
macro expansion (`BasicReverb` variant), in two accessor methods
(`as_basic_reverb_mut`, `as_basic_reverb`), and in a serialization test — all
requiring `#[allow(deprecated)]` annotations to compile silently (9 suppression
sites total).

The aliases exist solely to support existing `.prot`/`.mka` container files
whose embedded JSON still uses the string key `"BasicReverbSettings"`. The
`AudioEffect` macro handles this by mapping the `BasicReverb` deserialized
variant to the canonical `DelayReverb` variant at runtime via
`normalize_legacy_alias`.

## Consequences

* Nine `#[allow(deprecated)]` sites create noise and make it hard to spot
  genuinely problematic deprecated usage.
* The two accessor methods (`as_basic_reverb`, `as_basic_reverb_mut`) expose the
  old name on the public `AudioEffect` API, which is the very name consumers
  should be migrating away from.

## Fix

1. **Keep the serde deserialization bridge** — the `BasicReverb` enum variant in
   the `AudioEffect` macro must remain for backward-compatible JSON round-trips.
   The `normalize_legacy_alias` call that maps it to `DelayReverb` is correct and
   should stay.
2. **Delete the type aliases** from `basic_reverb/mod.rs`. They are not needed
   for the serde bridge and have no external callers in this repo.
3. **Delete `as_basic_reverb` and `as_basic_reverb_mut`** from `AudioEffect` (or
   replace them with a delegation to the canonical `as_delay_reverb` accessor if
   one exists). Verify no CLI or external consumer calls them first.
4. **Remove all `#[allow(deprecated)]` sites** that exist solely for the aliases.

After this change, the only trace of the old name will be the serde rename string
`"BasicReverbSettings"` inside the macro — which is the right place for it.
