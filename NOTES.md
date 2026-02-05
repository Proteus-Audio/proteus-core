# Playback Alignment Notes

## Mixing Alignment Rule
- When mixing selected tracks from a container (`.mka`/`.prot`), avoid emitting output unless every selected track has at least some buffered samples for the current time window.
- Mixing while a selected track buffer is empty causes dropouts and misalignment (samples from that track will be offset relative to others).
- The mixing stage should therefore ensure buffers are aligned: only advance the playhead for a mix when all active track buffers can contribute samples for that chunk.
