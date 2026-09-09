//! Create command handlers.

use clap::ArgMatches;
use log::error;
#[cfg(feature = "prot-mux")]
use proteus_lib::container::prot::PathsTrack;
#[cfg(feature = "prot-mux")]
use proteus_lib::container::prot::Prot;
#[cfg(feature = "prot-mux")]
use proteus_lib::dsp::effects::AudioEffect;
#[cfg(feature = "prot-mux")]
use proteus_lib::mux::{
    ProtMuxAttachment, ProtMuxInput, ProtMuxPhase, ProtMuxProgress, ProtMuxTrackInput,
};
#[cfg(feature = "prot-mux")]
use std::collections::BTreeMap;
#[cfg(feature = "prot-mux")]
use std::ffi::OsStr;
#[cfg(feature = "prot-mux")]
use std::fs;
#[cfg(feature = "prot-mux")]
use std::path::{Path, PathBuf};
#[cfg(feature = "prot-mux")]
use std::time::Duration;

use crate::project_files;

/// Handle `create effects-json`.
pub(crate) fn run_create_effects_json() -> i32 {
    let effects = project_files::default_effects_chain_enabled();
    match serde_json::to_string_pretty(&effects) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(err) => {
            error!("Failed to serialize effects: {}", err);
            -1
        }
    }
}

/// Handle `create prot`.
#[cfg(feature = "prot-mux")]
pub(crate) fn run_create_prot(args: &ArgMatches) -> i32 {
    match create_prot_from_args(args) {
        Ok(output) => {
            println!("Wrote {}", output.display());
            0
        }
        Err(err) => {
            error!("{}", err);
            -1
        }
    }
}

/// Handle `create prot` when muxing support is not compiled in.
#[cfg(not(feature = "prot-mux"))]
pub(crate) fn run_create_prot(_args: &ArgMatches) -> i32 {
    error!("This build does not include .prot creation support");
    -1
}

#[cfg(feature = "prot-mux")]
fn create_prot_from_args(args: &ArgMatches) -> Result<PathBuf, String> {
    let input_dir = Path::new(
        args.get_one::<String>("INPUT_DIR")
            .ok_or("missing input directory")?,
    );
    let output = PathBuf::from(
        args.get_one::<String>("OUTPUT")
            .ok_or("missing output path")?,
    );
    let force = args.get_flag("force");
    if output.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to overwrite it",
            output.display()
        ));
    }

    let directory =
        project_files::load_directory_playback_config(input_dir).map_err(|err| err.to_string())?;
    let effects = if args.get_flag("no-effects") {
        Vec::new()
    } else if let Some(path) = directory.effects_json_path.as_ref() {
        project_files::load_effects_json(path.to_string_lossy().as_ref())
            .map_err(|err| err.to_string())?
    } else {
        Vec::new()
    };

    let project = build_mux_project(input_dir, &directory.tracks, effects)?;
    let attachments = attachment_args(args)?
        .into_iter()
        .map(|path| read_attachment(&path))
        .collect::<Result<Vec<_>, _>>()?;
    let title = args.get_one::<String>("title").cloned();
    let mut progress = IndicatifMuxProgress::new();

    proteus_lib::mux::create_prot_with_progress(
        ProtMuxInput {
            output: output.clone(),
            tracks: project.mux_tracks,
            play_settings_json: project.play_settings_json,
            attachments,
            title,
            writing_app: Some("proteus-cli".to_string()),
        },
        &mut progress,
    )
    .map_err(|err| err.to_string())?;

    Prot::try_new(output.to_string_lossy().as_ref()).map_err(|err| err.to_string())?;
    Ok(output)
}

#[cfg(feature = "prot-mux")]
#[derive(Debug)]
struct MuxProject {
    mux_tracks: Vec<ProtMuxTrackInput>,
    play_settings_json: Vec<u8>,
}

#[cfg(feature = "prot-mux")]
fn build_mux_project(
    root: &Path,
    tracks: &[PathsTrack],
    effects: Vec<AudioEffect>,
) -> Result<MuxProject, String> {
    let mut ids_by_path = BTreeMap::new();
    let mut mux_tracks = Vec::new();
    for track in tracks {
        for path in &track.file_paths {
            if ids_by_path.contains_key(path) {
                continue;
            }
            let id = u32::try_from(mux_tracks.len() + 1).map_err(|err| err.to_string())?;
            ids_by_path.insert(path.clone(), id);
            let source_path = PathBuf::from(path);
            mux_tracks.push(ProtMuxTrackInput {
                title: Some(display_name_for_source(root, &source_path)),
                source_path,
            });
        }
    }

    let settings_tracks = tracks
        .iter()
        .map(|track| settings_track_for_paths(root, track, &ids_by_path))
        .collect::<Result<Vec<_>, _>>()?;

    let effects_json = serde_json::to_value(&effects).map_err(|err| err.to_string())?;
    let play_settings_json = serde_json::json!({
        "encoder_version": "3",
        "play_settings": {
            "effects": effects_json,
            "tracks": settings_tracks,
        }
    });
    let play_settings_json =
        serde_json::to_vec_pretty(&play_settings_json).map_err(|err| err.to_string())?;

    Ok(MuxProject {
        mux_tracks,
        play_settings_json,
    })
}

#[cfg(feature = "prot-mux")]
fn settings_track_for_paths(
    root: &Path,
    track: &PathsTrack,
    ids_by_path: &BTreeMap<String, u32>,
) -> Result<serde_json::Value, String> {
    let ids = track
        .file_paths
        .iter()
        .map(|path| {
            ids_by_path
                .get(path)
                .copied()
                .ok_or_else(|| format!("internal error: no mux track ID for {path}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let name = display_name_for_logical_track(root, track);

    Ok(serde_json::json!({
        "level": track.level,
        "pan": track.pan,
        "ids": ids,
        "name": name,
        "safe_name": safe_name(&name),
        "selections_count": track.selections_count.max(1),
        "shuffle_points": track.shuffle_points,
    }))
}

#[cfg(feature = "prot-mux")]
fn display_name_for_logical_track(root: &Path, track: &PathsTrack) -> String {
    let Some(first) = track.file_paths.first() else {
        return "track".to_string();
    };
    let path = Path::new(first);
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(rel)
        .to_string_lossy()
        .to_string()
}

#[cfg(feature = "prot-mux")]
fn display_name_for_source(root: &Path, source: &Path) -> String {
    source
        .strip_prefix(root)
        .unwrap_or(source)
        .to_string_lossy()
        .to_string()
}

#[cfg(feature = "prot-mux")]
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(feature = "prot-mux")]
fn attachment_args(args: &ArgMatches) -> Result<Vec<PathBuf>, String> {
    Ok(args
        .get_many::<String>("attach")
        .map(|values| values.map(PathBuf::from).collect())
        .unwrap_or_default())
}

#[cfg(feature = "prot-mux")]
fn read_attachment(path: &Path) -> Result<ProtMuxAttachment, String> {
    let data =
        fs::read(path).map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("attachment path has no filename: {}", path.display()))?;
    Ok(ProtMuxAttachment::new(
        file_name,
        infer_mime_type(path),
        data,
    ))
}

#[cfg(feature = "prot-mux")]
fn infer_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("lrc") => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(feature = "prot-mux")]
struct IndicatifMuxProgress {
    bar: indicatif::ProgressBar,
}

#[cfg(feature = "prot-mux")]
impl IndicatifMuxProgress {
    fn new() -> Self {
        let bar = indicatif::ProgressBar::new(0);
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)",
            )
            .expect("progress template is valid")
            .progress_chars("#>-"),
        );
        bar.set_message("starting");
        bar.enable_steady_tick(Duration::from_millis(100));
        Self { bar }
    }
}

#[cfg(feature = "prot-mux")]
impl ProtMuxProgress for IndicatifMuxProgress {
    fn set_phase(&mut self, phase: ProtMuxPhase) {
        match phase {
            ProtMuxPhase::Starting => self.bar.set_message("scanning project"),
            ProtMuxPhase::Transcoding(path) => {
                self.bar
                    .set_message(format!("transcoding {}", path.display()));
            }
            ProtMuxPhase::OrderingPackets => self.bar.set_message("ordering packets"),
            ProtMuxPhase::PreparingOutput => self.bar.set_message("preparing output"),
            ProtMuxPhase::EmbeddingPlaySettings => self.bar.set_message("embedding play settings"),
            ProtMuxPhase::EmbeddingAttachment(name) => {
                self.bar.set_message(format!("embedding {name}"));
            }
            ProtMuxPhase::BuildingMuxer => self.bar.set_message("building muxer"),
            ProtMuxPhase::WritingPackets => self.bar.set_message("writing packets"),
            ProtMuxPhase::Finalizing => self.bar.set_message("finalizing output"),
            ProtMuxPhase::Complete(path) => {
                self.bar
                    .finish_with_message(format!("wrote {}", path.display()));
            }
        }
    }

    fn increment(&mut self, units: u64) {
        self.bar.inc(units);
    }

    fn add_work(&mut self, units: u64) {
        let next = self.bar.length().unwrap_or(0).saturating_add(units);
        self.bar.set_length(next);
    }
}

#[cfg(test)]
mod tests {
    use super::run_create_effects_json;
    #[cfg(feature = "prot-mux")]
    use proteus_lib::container::prot::PathsTrack;
    #[cfg(feature = "prot-mux")]
    use std::path::Path;

    #[test]
    fn create_effects_json_returns_success() {
        let code = run_create_effects_json();
        assert_eq!(code, 0);
    }

    #[cfg(feature = "prot-mux")]
    #[test]
    fn build_mux_project_preserves_group_settings_and_ids() {
        let tracks = vec![PathsTrack {
            file_paths: vec![
                "/tmp/proteus/a/one.wav".to_string(),
                "/tmp/proteus/a/two.wav".to_string(),
            ],
            level: 0.5,
            pan: -0.25,
            selections_count: 2,
            shuffle_points: vec!["00:01".to_string()],
        }];

        let project =
            super::build_mux_project(Path::new("/tmp/proteus"), &tracks, Vec::new()).unwrap();
        assert_eq!(project.mux_tracks.len(), 2);
        let json: serde_json::Value = serde_json::from_slice(&project.play_settings_json).unwrap();
        assert_eq!(json["encoder_version"], "3");
        assert_eq!(
            json["play_settings"]["tracks"][0]["ids"],
            serde_json::json!([1, 2])
        );
        assert_eq!(json["play_settings"]["tracks"][0]["level"], 0.5);
        assert_eq!(json["play_settings"]["tracks"][0]["pan"], -0.25);
        assert_eq!(json["play_settings"]["tracks"][0]["selections_count"], 2);
        assert_eq!(
            json["play_settings"]["tracks"][0]["shuffle_points"],
            serde_json::json!(["00:01"])
        );
    }

    #[cfg(feature = "prot-mux")]
    #[test]
    fn create_prot_refuses_to_overwrite_without_force() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out.prot");
        std::fs::write(&output, b"existing").unwrap();
        let input = temp.path().join("missing-input");
        let args = crate::cli::args::build_cli()
            .try_get_matches_from([
                "prot",
                "create",
                "prot",
                input.to_str().unwrap(),
                output.to_str().unwrap(),
            ])
            .unwrap();
        let (_, create_args) = args.subcommand().unwrap();
        let (_, prot_args) = create_args.subcommand().unwrap();

        let err = super::create_prot_from_args(prot_args).unwrap_err();
        assert!(err.contains("--force"));
    }
}
