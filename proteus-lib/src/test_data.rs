//! Local test data paths used by development helpers.

fn get_double_vec_of_mp3s() -> Vec<Vec<String>> {
    vec![
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_bgclar1.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_bgclar2.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_bgclar3.mp3".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_clar1.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_clar2.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_clar3.mp3".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_piano1.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_piano2.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_piano3.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_piano4.mp3".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_rythmn1.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_rythmn2.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_rythmn3.mp3".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/op_rythmn4.mp3".to_string(),
        ],
    ]
}

fn get_double_vec_of_wavs() -> Vec<Vec<String>> {
    vec![
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_bgclar1.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_bgclar2.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_bgclar3.wav".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_clar1.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_clar2.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_clar3.wav".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_piano1.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_piano2.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_piano3.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_piano4.wav".to_string(),
        ],
        vec![
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_rythmn1.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_rythmn2.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_rythmn3.wav".to_string(),
            "/Users/innocentsmith/Dev/tauri/proteus-author/dev-assets/24bit_wav/op_rythmn4.wav".to_string(),
        ],
    ]
}

/// Convenience wrapper for local test asset paths.
///
/// Note: Paths are machine-specific and intended for local development only.
pub struct TestData {
    pub mp3s: Vec<Vec<String>>,
    pub wavs: Vec<Vec<String>>,   
}

impl TestData {
    /// Build a new set of local test asset paths.
    pub fn new() -> Self {
        Self {
            mp3s: get_double_vec_of_mp3s(),
            wavs: get_double_vec_of_wavs(),
        }
    }
}
