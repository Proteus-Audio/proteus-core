# Proteus Project History

Proteus started as an exploration of procedural song playback where each play can select different takes per musical part.

Early prototypes focused on generating fresh composites from recorded stems. The project then moved through multiple desktop approaches while refining container and playback architecture:

- initial experiments with script-driven audio composition
- Flutter desktop tooling for authoring and playback workflows
- Electron-based rewrite attempt
- migration to a Rust-centered workspace with `proteus-lib` and `proteus-cli`

The current repository is the Rust workspace runtime. Historical UI/runtime migration notes are kept here to avoid mixing legacy narrative with current architecture contracts.
