//! Duration probing with source provenance.

mod adts;
mod aiff;
mod flac;
mod matroska;
mod mp3;
mod mp4;
mod ogg;
mod wav;

use std::{collections::HashMap, fmt};

use symphonia::core::formats::Track;

use super::get_time_from_frames;

/// Where a probed duration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationSourceKind {
    /// Reliable container, stream-header, sample-table, or final-timestamp structure.
    Structural,
    /// Codec parameters with a known frame count and time base.
    FrameCount,
    /// Free-form metadata tag such as `DURATION`.
    Tag,
    /// A scan of packet/page/frame headers without decoding audio payloads.
    HeaderScan,
    /// A full packet timestamp scan.
    PacketScan,
}

impl fmt::Display for DurationSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structural => write!(f, "structural"),
            Self::FrameCount => write!(f, "frame-count"),
            Self::Tag => write!(f, "tag"),
            Self::HeaderScan => write!(f, "header-scan"),
            Self::PacketScan => write!(f, "packet-scan"),
        }
    }
}

/// Detailed duration probe result for one track.
#[derive(Debug, Clone, PartialEq)]
pub struct DurationDetail {
    /// Symphonia track id.
    pub track_id: u32,
    /// Duration in seconds.
    pub seconds: f64,
    /// Source used to derive the duration.
    pub source: DurationSourceKind,
    /// Whether the value should be treated as exact rather than estimated.
    pub exact: bool,
}

impl DurationDetail {
    pub(super) fn new(
        track_id: u32,
        seconds: f64,
        source: DurationSourceKind,
        exact: bool,
    ) -> Self {
        Self {
            track_id,
            seconds,
            source,
            exact,
        }
    }
}

pub(super) fn details_to_map(details: &[DurationDetail]) -> HashMap<u32, f64> {
    details
        .iter()
        .map(|detail| (detail.track_id, detail.seconds))
        .collect()
}

pub(super) fn probe_structural(file_path: &str, tracks: &[Track]) -> Option<Vec<DurationDetail>> {
    if let Some(durations) = ogg::probe_durations(file_path, tracks) {
        return Some(map_to_details(
            durations,
            DurationSourceKind::Structural,
            true,
        ));
    }
    if let Some(details) = flac::probe(file_path, tracks) {
        return Some(details);
    }
    if let Some(details) = wav::probe(file_path, tracks) {
        return Some(details);
    }
    if let Some(details) = aiff::probe(file_path, tracks) {
        return Some(details);
    }
    if let Some(details) = matroska::probe(file_path, tracks) {
        return Some(details);
    }
    if let Some(details) = mp4::probe(file_path, tracks) {
        return Some(details);
    }
    if let Some(details) = mp3::probe(file_path, tracks) {
        return Some(details);
    }
    adts::probe(file_path, tracks)
}

pub(super) fn probe_standalone_structural(file_path: &str) -> Option<Vec<DurationDetail>> {
    if let Some(details) = flac::probe_standalone(file_path) {
        return Some(details);
    }
    if let Some(details) = wav::probe_standalone(file_path) {
        return Some(details);
    }
    if let Some(details) = aiff::probe_standalone(file_path) {
        return Some(details);
    }
    if let Some(details) = mp4::probe_standalone(file_path) {
        return Some(details);
    }
    if let Some(details) = mp3::probe_standalone(file_path) {
        return Some(details);
    }
    adts::probe_standalone(file_path)
}

pub(super) fn probe_frame_counts(tracks: &[Track]) -> Option<Vec<DurationDetail>> {
    let details = tracks
        .iter()
        .filter_map(|track| {
            let seconds = get_time_from_frames(&track.codec_params);
            if seconds > 0.0 {
                Some(DurationDetail::new(
                    track.id,
                    seconds,
                    DurationSourceKind::FrameCount,
                    true,
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if details.is_empty() {
        None
    } else {
        Some(details)
    }
}

pub(super) fn tag_details(tag_durations: &[f64], tracks: &[Track]) -> Option<Vec<DurationDetail>> {
    let details = tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| {
            let seconds = *tag_durations.get(index)?;
            if seconds > 0.0 {
                Some(DurationDetail::new(
                    track.id,
                    seconds,
                    DurationSourceKind::Tag,
                    false,
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if details.is_empty() {
        None
    } else {
        Some(details)
    }
}

pub(super) fn choose_track_details(
    structural: Option<Vec<DurationDetail>>,
    frame_counts: Option<Vec<DurationDetail>>,
    tags: Option<Vec<DurationDetail>>,
) -> Option<Vec<DurationDetail>> {
    structural.or(frame_counts).or(tags)
}

pub(super) fn packet_scan_details(durations: HashMap<u32, f64>) -> Vec<DurationDetail> {
    map_to_details(durations, DurationSourceKind::PacketScan, true)
}

fn map_to_details(
    durations: HashMap<u32, f64>,
    source: DurationSourceKind,
    exact: bool,
) -> Vec<DurationDetail> {
    let mut details = durations
        .into_iter()
        .filter(|(_, seconds)| *seconds > 0.0)
        .map(|(track_id, seconds)| DurationDetail::new(track_id, seconds, source, exact))
        .collect::<Vec<_>>();
    details.sort_by(|a, b| a.track_id.cmp(&b.track_id));
    details
}

pub(super) fn single_track_detail(
    seconds: f64,
    source: DurationSourceKind,
    exact: bool,
    tracks: &[Track],
) -> Option<Vec<DurationDetail>> {
    if seconds <= 0.0 {
        return None;
    }
    let details = tracks
        .iter()
        .map(|track| DurationDetail::new(track.id, seconds, source, exact))
        .collect::<Vec<_>>();
    if details.is_empty() {
        None
    } else {
        Some(details)
    }
}

fn standalone_detail(
    seconds: f64,
    source: DurationSourceKind,
    exact: bool,
) -> Option<Vec<DurationDetail>> {
    if seconds <= 0.0 {
        None
    } else {
        Some(vec![DurationDetail::new(0, seconds, source, exact)])
    }
}

fn extension_lc(file_path: &str) -> Option<String> {
    std::path::Path::new(file_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::codecs::CodecParameters;

    fn track(id: u32) -> Track {
        Track {
            id,
            codec_params: CodecParameters {
                sample_rate: Some(44_100),
                ..Default::default()
            },
            language: None,
        }
    }

    #[test]
    fn tag_details_mark_values_as_inexact() {
        let details = tag_details(&[1.25], &[track(7)]).expect("details");

        assert_eq!(details[0].track_id, 7);
        assert_eq!(details[0].source, DurationSourceKind::Tag);
        assert!(!details[0].exact);
    }

    #[test]
    fn reliable_duration_sources_are_preferred_over_stale_tags_for_all_families() {
        let families = [
            ("ogg", DurationSourceKind::Structural),
            ("flac", DurationSourceKind::Structural),
            ("wav", DurationSourceKind::Structural),
            ("aiff", DurationSourceKind::Structural),
            ("mp3", DurationSourceKind::Structural),
            ("mp3-header-scan", DurationSourceKind::HeaderScan),
            ("mp4", DurationSourceKind::Structural),
            ("matroska", DurationSourceKind::Structural),
            ("adts", DurationSourceKind::HeaderScan),
        ];

        for (family, source) in families {
            let reliable = vec![DurationDetail::new(0, 1.0, source, true)];
            let stale_tag = vec![DurationDetail::new(
                0,
                13_442.888,
                DurationSourceKind::Tag,
                false,
            )];

            let details = choose_track_details(Some(reliable), None, Some(stale_tag))
                .expect("duration details");

            assert_eq!(details[0].seconds, 1.0, "{}", family);
            assert_eq!(details[0].source, source, "{}", family);
        }
    }

    #[test]
    fn frame_counts_are_preferred_over_stale_tags() {
        let frame_count = vec![DurationDetail::new(
            0,
            2.0,
            DurationSourceKind::FrameCount,
            true,
        )];
        let stale_tag = vec![DurationDetail::new(
            0,
            13_442.888,
            DurationSourceKind::Tag,
            false,
        )];

        let details =
            choose_track_details(None, Some(frame_count), Some(stale_tag)).expect("details");

        assert_eq!(details[0].seconds, 2.0);
        assert_eq!(details[0].source, DurationSourceKind::FrameCount);
    }
}
