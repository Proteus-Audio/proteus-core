mod orchestration;

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
