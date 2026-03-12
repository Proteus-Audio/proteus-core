# SI-08: Public API Documentation Coverage Incomplete

## Files affected

This issue spans multiple public `proteus-lib` modules, with the heaviest concentrations in:

| File | Notes |
|---|---|
| `proteus-lib/src/container/info/mod.rs` | public error variants and public data fields undocumented |
| `proteus-lib/src/container/play_settings/mod.rs` | public settings schema fields and enum variants undocumented |
| `proteus-lib/src/dsp/effects/*` | many public effect settings/effect fields undocumented |
| `proteus-lib/src/playback/engine/mod.rs` | public config/update structs undocumented |
| `proteus-lib/src/playback/engine/state.rs` | public runtime settings/metrics fields undocumented |
| `proteus-lib/src/playback/player/mod.rs` | public enum variants and snapshot fields undocumented |
| `proteus-lib/src/peaks/mod.rs` | public peak data fields undocumented |

---

## Current state

`cargo rustdoc -p proteus-lib --lib -- -D missing_docs` currently reports **212 missing
documentation errors** across public modules, structs, fields, enum variants, and methods.
The check covers only `pub` items visible outside the crate; `pub(crate)` items are excluded.

The sweep also exposed several broken or private intra-doc links. Some of those were fixed in
place as part of this pass, but the larger documentation gap remains open.

### Specific style-guide violations

- Public items in `proteus-lib` are missing required `///` docs
- Public module surfaces are inconsistent about `# Arguments`, `# Returns`, `# Errors`, and
  `# Panics` sections
- Public schema types expose undocumented fields, which makes serialized formats harder to use
  correctly

### Why this matters

The style guide treats docs as part of the public contract. Right now, consumers cannot reliably
discover what public settings fields mean, what enum variants represent, or what runtime metrics
are safe to depend on. The missing docs also block using compiler-enforced documentation checks to
prevent regression.

### Recommended remediation

1. Enable a staged documentation pass by area rather than file-by-file opportunism:
   - `container/*`
   - `dsp/effects/*`
   - `playback/*`
   - `peaks/*` and `tools/*`
2. Document public data types first:
   - error enums and their variants
   - public config/settings structs and each field
   - public snapshot/result types and each field
3. Standardize function docs to include structured sections whenever applicable
4. Once the crate is clean, gate regressions with `#![deny(missing_docs)]` at the crate level or
   an equivalent CI rustdoc check

### Acceptance criteria

- [ ] `cargo rustdoc -p proteus-lib --lib -- -D missing_docs` passes
- [ ] Public functions returning `Result` include `# Errors`
- [ ] Public panicking constructors include `# Panics`
- [ ] Public schema/config structs document every exported field
- [ ] Broken intra-doc links are eliminated from public docs

## Status

Open.
