use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::container::prot::{PathsTrack, Prot};
use crate::dsp::effects::AudioEffect;
use crate::playback::player::{Player, PlayerInitOptions, PlayerSource};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ShuffleScheduleFile {
    Tracks(Vec<JsonPathsTrack>),
    Wrapped { tracks: Vec<JsonPathsTrack> },
}

#[derive(Debug, Deserialize)]
struct JsonPathsTrack {
    file_paths: Vec<String>,
    #[serde(default = "default_level")]
    level: f32,
    #[serde(default)]
    pan: f32,
    #[serde(default = "default_selections_count")]
    selections_count: u32,
    #[serde(default)]
    shuffle_points: Vec<String>,
}

fn default_level() -> f32 {
    1.0
}

fn default_selections_count() -> u32 {
    1
}

#[test]
fn smoke_loads_and_initializes_32bit_mp3() {
    let fixture = test_audio_path("test-32bit.mp3");
    assert!(fixture.is_file(), "missing fixture: {}", fixture.display());

    let track = PathsTrack::new_from_file_paths(vec![fixture.display().to_string()]);
    let player = Player::try_from_source_with_options(
        PlayerSource::FilePaths(vec![track]),
        PlayerInitOptions::default(),
    )
    .expect("32-bit mp3 fixture should initialize");

    let info = player.audio_info();
    assert!(info.sample_rate > 0, "sample rate should be populated");
    assert!(info.channels > 0, "channel count should be populated");
    assert!(player.get_shuffle_schedule().len() == 1);
    assert!(
        !player.get_ids().is_empty(),
        "player should expose at least one source id"
    );

    player.stop();
}

#[test]
fn smoke_loads_and_initializes_demo_shuffle_points_container() {
    let fixture = test_audio_path("demo_shuffle_points.prot");
    assert!(fixture.is_file(), "missing fixture: {}", fixture.display());

    let prot = Prot::try_new(&fixture.display().to_string())
        .expect("demo_shuffle_points.prot should parse as a container");
    assert!(
        prot.info.sample_rate > 0,
        "container sample rate should be populated"
    );
    assert!(
        prot.info.channels > 0,
        "container channel count should be populated"
    );
    assert!(
        !prot.get_shuffle_schedule().is_empty(),
        "container should resolve a runtime shuffle schedule"
    );

    let player = Player::try_from_source_with_options(
        PlayerSource::ContainerPath(fixture.display().to_string()),
        PlayerInitOptions::default(),
    )
    .expect("container-backed player should initialize");

    assert!(player.audio_info().sample_rate > 0);
    assert!(player.audio_info().channels > 0);
    assert!(
        !player.get_shuffle_schedule().is_empty(),
        "player should expose the resolved container schedule"
    );
    assert!(
        !player.get_ids().is_empty(),
        "player should expose resolved track ids"
    );

    player.stop();
}

#[test]
fn smoke_loads_and_initializes_directory_backed_24bit_wav_session() {
    let fixture_root = test_audio_path("24bit_wav");
    assert!(
        fixture_root.is_dir(),
        "missing fixture directory: {}",
        fixture_root.display()
    );

    let tracks = load_paths_tracks_from_shuffle_schedule(&fixture_root);
    assert_eq!(tracks.len(), 4, "expected four logical tracks from fixture");

    let effects = load_effects_json(&fixture_root.join("effects_chain.json"));
    assert!(
        !effects.is_empty(),
        "directory fixture should include a non-empty effects chain"
    );

    let prot = Prot::new_from_file_paths(tracks.clone());
    assert!(
        prot.info.sample_rate > 0,
        "directory sample rate should be populated"
    );
    assert!(
        prot.info.channels > 0,
        "directory channel count should be populated"
    );
    assert!(
        !prot.get_shuffle_schedule().is_empty(),
        "directory-backed prot should produce a runtime schedule"
    );

    let mut player = Player::try_from_source_with_options(
        PlayerSource::FilePaths(tracks),
        PlayerInitOptions::default(),
    )
    .expect("directory-backed player should initialize");
    player.set_effects(effects);

    assert!(player.audio_info().sample_rate > 0);
    assert!(player.audio_info().channels > 0);
    assert!(
        !player.get_shuffle_schedule().is_empty(),
        "player should expose the directory-backed schedule"
    );
    assert_eq!(
        player.get_ids().len(),
        4,
        "expected one id per logical track"
    );
    assert!(
        !player.get_effect_names().is_empty(),
        "player should accept the fixture effect chain"
    );

    player.stop();
}

fn test_audio_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("test_audio")
        .join(relative)
}

fn load_paths_tracks_from_shuffle_schedule(root: &Path) -> Vec<PathsTrack> {
    let schedule_path = root.join("shuffle_schedule.json");
    let raw = fs::read_to_string(&schedule_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", schedule_path.display()));
    let parsed: ShuffleScheduleFile = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", schedule_path.display()));

    let tracks = match parsed {
        ShuffleScheduleFile::Tracks(tracks) => tracks,
        ShuffleScheduleFile::Wrapped { tracks } => tracks,
    };

    tracks
        .into_iter()
        .map(|track| PathsTrack {
            file_paths: track
                .file_paths
                .into_iter()
                .map(|path| root.join(path).display().to_string())
                .collect(),
            level: track.level,
            pan: track.pan,
            selections_count: track.selections_count.max(1),
            shuffle_points: track.shuffle_points,
        })
        .collect()
}

fn load_effects_json(path: &Path) -> Vec<AudioEffect> {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}
