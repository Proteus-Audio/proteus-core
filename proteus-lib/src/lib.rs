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
pub(crate) mod logging;
pub mod peaks;
pub mod playback;
pub mod test_data;
pub mod tools;
mod track;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_exports_expected_top_level_modules() {
        use crate::{audio, container, diagnostics, dsp, peaks, playback, test_data, tools};

        let _ = audio::buffer::init_buffer_map();
        let _ = core::mem::size_of::<container::info::Info>();
        let _ = core::mem::size_of::<diagnostics::reporter::Report>();
        let _ = core::mem::size_of::<dsp::effects::GainEffect>();
        let _ = core::mem::size_of::<peaks::GetPeaksOptions>();
        let _ = core::mem::size_of::<playback::player::PlayerState>();
        let _ = test_data::TestData::new();
        let _ = core::mem::size_of::<tools::timer::Timer>();
    }
}
