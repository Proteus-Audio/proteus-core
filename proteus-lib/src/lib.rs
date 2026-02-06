//! # Proteus Audio Library
//!
//! `proteus-lib` provides container parsing, playback, and DSP utilities used by
//! the Proteus ecosystem. It is designed to be embedded in GUI apps and CLIs.
//!
//! **Key areas**
//! - `container`: `.prot`/`.mka` parsing, play settings, and duration scanning.
//! - `playback`: real-time mixing engine and a high-level [`playback::player::Player`].
//! - `dsp`: convolution and impulse response utilities for reverb.
//! - `diagnostics`: optional benchmarks and metrics reporting.

pub mod audio;
pub mod container;
pub mod diagnostics;
pub mod dsp;
pub mod peaks;
pub mod playback;
pub mod test_data;
pub mod tools;
mod track;
