//! Directory-backed multi-file playback config helpers for the CLI.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use proteus_lib::container::prot::PathsTrack;
use proteus_lib::dsp::effects::{
    AudioEffect, CompressorEffect, ConvolutionReverbEffect, DelayReverbEffect,
    DiffusionReverbEffect, DistortionEffect, GainEffect, HighPassFilterEffect, LimiterEffect,
    LowPassFilterEffect, MultibandEqEffect, PanEffect,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct DirectoryPlaybackConfig {
    pub tracks: Vec<PathsTrack>,
    pub effects_json_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonPathsTrack {
    pub file_paths: Vec<String>,
    #[serde(default = "default_level")]
    pub level: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default = "default_selections_count")]
    pub selections_count: u32,
    #[serde(default)]
    pub shuffle_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum ShuffleScheduleFile {
    Tracks(Vec<JsonPathsTrack>),
    Wrapped { tracks: Vec<JsonPathsTrack> },
}

fn default_level() -> f32 {
    1.0
}

fn default_selections_count() -> u32 {
    1
}

pub fn load_directory_playback_config(
    root: &Path,
) -> std::result::Result<DirectoryPlaybackConfig, String> {
    let shuffle_path = root.join("shuffle_schedule.json");
    let effects_path = root.join("effects_chain.json");

    let tracks = if shuffle_path.is_file() {
        load_paths_tracks_json(root, &shuffle_path)?
    } else {
        discover_paths_tracks_from_directory(root)?
    };

    if tracks.is_empty() {
        return Err(format!(
            "No audio files found under directory: {}",
            root.display()
        ));
    }

    Ok(DirectoryPlaybackConfig {
        tracks,
        effects_json_path: effects_path.is_file().then_some(effects_path),
    })
}

pub fn load_effects_json(path: &str) -> std::result::Result<Vec<AudioEffect>, String> {
    let raw =
        fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse json: {}", err))
}

pub fn write_init_files(root: &Path) -> std::result::Result<(), String> {
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", root.display()));
    }

    let shuffle_tracks = discover_paths_tracks_json_from_directory(root)?;
    if shuffle_tracks.is_empty() {
        return Err(format!(
            "No audio files found under directory: {}",
            root.display()
        ));
    }

    let shuffle_path = root.join("shuffle_schedule.json");
    let effects_path = root.join("effects_chain.json");

    let shuffle_json = serde_json::to_string_pretty(&ShuffleScheduleFile::Wrapped {
        tracks: shuffle_tracks,
    })
    .map_err(|err| format!("failed to serialize shuffle_schedule.json: {}", err))?;
    let effects_json = serde_json::to_string_pretty(&default_effects_chain_disabled())
        .map_err(|err| format!("failed to serialize effects_chain.json: {}", err))?;

    fs::write(&shuffle_path, shuffle_json)
        .map_err(|err| format!("failed to write {}: {}", shuffle_path.display(), err))?;
    fs::write(&effects_path, effects_json)
        .map_err(|err| format!("failed to write {}: {}", effects_path.display(), err))?;

    println!("Wrote {}", shuffle_path.display());
    println!("Wrote {}", effects_path.display());
    Ok(())
}

pub fn default_effects_chain_enabled() -> Vec<AudioEffect> {
    vec![
        AudioEffect::ConvolutionReverb(ConvolutionReverbEffect::default()),
        AudioEffect::DiffusionReverb(DiffusionReverbEffect::default()),
        AudioEffect::DelayReverb(DelayReverbEffect::default()),
        AudioEffect::LowPassFilter(LowPassFilterEffect::default()),
        AudioEffect::HighPassFilter(HighPassFilterEffect::default()),
        AudioEffect::Distortion(DistortionEffect::default()),
        AudioEffect::Gain(GainEffect::default()),
        AudioEffect::Compressor(CompressorEffect::default()),
        AudioEffect::Limiter(LimiterEffect::default()),
        AudioEffect::MultibandEq(MultibandEqEffect::default()),
        AudioEffect::Pan(PanEffect::default()),
    ]
}

pub fn default_effects_chain_disabled() -> Vec<AudioEffect> {
    default_effects_chain_enabled()
        .into_iter()
        .map(disable_effect)
        .collect()
}

fn disable_effect(mut effect: AudioEffect) -> AudioEffect {
    match &mut effect {
        AudioEffect::DelayReverb(e) => e.enabled = false,
        #[allow(deprecated)]
        AudioEffect::BasicReverb(e) => e.enabled = false,
        AudioEffect::DiffusionReverb(e) => e.enabled = false,
        AudioEffect::ConvolutionReverb(e) => e.enabled = false,
        AudioEffect::LowPassFilter(e) => e.enabled = false,
        AudioEffect::HighPassFilter(e) => e.enabled = false,
        AudioEffect::Distortion(e) => e.enabled = false,
        AudioEffect::Gain(e) => e.enabled = false,
        AudioEffect::Compressor(e) => e.enabled = false,
        AudioEffect::Limiter(e) => e.enabled = false,
        AudioEffect::MultibandEq(e) => e.enabled = false,
        AudioEffect::Pan(e) => e.enabled = false,
    }
    effect
}

fn load_paths_tracks_json(
    root: &Path,
    path: &Path,
) -> std::result::Result<Vec<PathsTrack>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let parsed: ShuffleScheduleFile = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {}", path.display(), err))?;
    let json_tracks = match parsed {
        ShuffleScheduleFile::Tracks(tracks) => tracks,
        ShuffleScheduleFile::Wrapped { tracks } => tracks,
    };

    let mut tracks = Vec::with_capacity(json_tracks.len());
    for track in json_tracks {
        let mut resolved = Vec::with_capacity(track.file_paths.len());
        for entry in track.file_paths {
            let candidate = Path::new(&entry);
            let resolved_path = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                root.join(candidate)
            };
            if !resolved_path.is_file() {
                return Err(format!(
                    "shuffle_schedule.json references missing file: {}",
                    resolved_path.display()
                ));
            }
            resolved.push(resolved_path.to_string_lossy().to_string());
        }
        tracks.push(PathsTrack {
            file_paths: resolved,
            level: track.level,
            pan: track.pan,
            selections_count: track.selections_count.max(1),
            shuffle_points: track.shuffle_points,
        });
    }

    Ok(tracks)
}

fn discover_paths_tracks_json_from_directory(
    root: &Path,
) -> std::result::Result<Vec<JsonPathsTrack>, String> {
    let grouped = discover_audio_groups(root)?;
    let mut tracks = Vec::new();
    for (_group, files) in grouped {
        if files.is_empty() {
            continue;
        }
        let mut rel_files = Vec::with_capacity(files.len());
        for path in files {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            rel_files.push(rel.to_string_lossy().to_string());
        }
        tracks.push(JsonPathsTrack {
            file_paths: rel_files,
            level: 1.0,
            pan: 0.0,
            selections_count: 1,
            shuffle_points: Vec::new(),
        });
    }
    Ok(tracks)
}

fn discover_paths_tracks_from_directory(
    root: &Path,
) -> std::result::Result<Vec<PathsTrack>, String> {
    let grouped = discover_audio_groups(root)?;
    let mut tracks = Vec::new();
    for (_group, files) in grouped {
        let abs_files: Vec<String> = files
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if !abs_files.is_empty() {
            tracks.push(PathsTrack::new_from_file_paths(abs_files));
        }
    }
    Ok(tracks)
}

fn discover_audio_groups(
    root: &Path,
) -> std::result::Result<BTreeMap<String, Vec<PathBuf>>, String> {
    let mut grouped: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    collect_audio_files_recursive(root, root, &mut grouped)
        .map_err(|err| format!("failed to scan {}: {}", root.display(), err))?;

    for files in grouped.values_mut() {
        files.sort();
        files.dedup();
    }

    Ok(grouped)
}

fn collect_audio_files_recursive(
    root: &Path,
    dir: &Path,
    grouped: &mut BTreeMap<String, Vec<PathBuf>>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files_recursive(root, &path, grouped)?;
            continue;
        }
        if !is_supported_audio_file(&path) {
            continue;
        }
        let parent = path.parent().unwrap_or(root);
        let rel_parent = parent.strip_prefix(root).unwrap_or(parent);
        let key = if rel_parent.as_os_str().is_empty() {
            ".".to_string()
        } else {
            rel_parent.to_string_lossy().to_string()
        };
        grouped.entry(key).or_default().push(path);
    }
    Ok(())
}

fn is_supported_audio_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("wav")
            | Some("wave")
            | Some("flac")
            | Some("aif")
            | Some("aiff")
            | Some("mp3")
            | Some("m4a")
            | Some("aac")
            | Some("ogg")
            | Some("opus")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn load_effects_json_parses_effects() {
        let effects = default_effects_chain_enabled();
        let json = serde_json::to_string_pretty(&effects).expect("serialize effects");

        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(json.as_bytes()).expect("write json");

        let loaded = load_effects_json(file.path().to_str().unwrap()).expect("load json");
        assert_eq!(loaded.len(), effects.len());
    }

    #[test]
    fn write_init_files_creates_json_outputs() {
        let dir = tempdir().expect("tempdir");
        let group = dir.path().join("piano");
        fs::create_dir_all(&group).expect("mkdir");
        fs::write(group.join("a.wav"), []).expect("wav");
        fs::write(group.join("b.wav"), []).expect("wav");

        write_init_files(dir.path()).expect("write init files");

        assert!(dir.path().join("shuffle_schedule.json").is_file());
        assert!(dir.path().join("effects_chain.json").is_file());
    }
}
