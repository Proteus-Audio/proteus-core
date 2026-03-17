//! Legacy track buffering modules.
//!
//! These modules are superseded by the decode workers in
//! `playback::engine::mix::runner::decode`, which share EOS detection and
//! shutdown logic through [`decode::decode_and_forward_packet`].
