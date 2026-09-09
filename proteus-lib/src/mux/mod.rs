//! Feature-gated helpers for creating `.prot`/`.mka` containers.

mod ogg;
mod transcode;

use std::fs::File;
use std::path::{Path, PathBuf};

use proteus_mux::{Attachment, MkaMuxer, VorbisTrackConfig};

use transcode::transcode_to_vorbis;

/// Result type returned by muxing operations.
pub type Result<T> = std::result::Result<T, MuxError>;

/// Failure while creating a Proteus container.
#[derive(Debug)]
pub enum MuxError {
    /// Filesystem IO failed.
    Io(std::io::Error),
    /// Symphonia failed while probing or decoding source audio.
    Symphonia(symphonia::core::errors::Error),
    /// Vorbis encoding failed.
    Vorbis(vorbis_rs::VorbisError),
    /// The muxer rejected track, attachment, or packet data.
    ProteusMux(proteus_mux::MuxError),
    /// Integer conversion failed while assigning IDs or progress units.
    IntConversion(std::num::TryFromIntError),
    /// The input was structurally invalid.
    InvalidInput(String),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {}", err),
            Self::Symphonia(err) => write!(f, "audio decode error: {}", err),
            Self::Vorbis(err) => write!(f, "vorbis encode error: {}", err),
            Self::ProteusMux(err) => write!(f, "mux error: {}", err),
            Self::IntConversion(err) => write!(f, "integer conversion error: {}", err),
            Self::InvalidInput(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for MuxError {}

impl From<std::io::Error> for MuxError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<symphonia::core::errors::Error> for MuxError {
    fn from(value: symphonia::core::errors::Error) -> Self {
        Self::Symphonia(value)
    }
}

impl From<vorbis_rs::VorbisError> for MuxError {
    fn from(value: vorbis_rs::VorbisError) -> Self {
        Self::Vorbis(value)
    }
}

impl From<proteus_mux::MuxError> for MuxError {
    fn from(value: proteus_mux::MuxError) -> Self {
        Self::ProteusMux(value)
    }
}

impl From<std::num::TryFromIntError> for MuxError {
    fn from(value: std::num::TryFromIntError) -> Self {
        Self::IntConversion(value)
    }
}

/// Input for creating a Proteus container.
#[derive(Debug, Clone)]
pub struct ProtMuxInput {
    /// Output `.prot`/`.mka` path.
    pub output: PathBuf,
    /// Source audio files. Each source becomes one muxed audio track.
    pub tracks: Vec<ProtMuxTrackInput>,
    /// Serialized `play_settings.json` payload to embed.
    pub play_settings_json: Vec<u8>,
    /// Additional attachments to embed.
    pub attachments: Vec<ProtMuxAttachment>,
    /// Optional container title.
    pub title: Option<String>,
    /// Optional muxer writing application name.
    pub writing_app: Option<String>,
}

/// One source audio file to mux as a Vorbis track.
#[derive(Debug, Clone)]
pub struct ProtMuxTrackInput {
    /// Source audio file path.
    pub source_path: PathBuf,
    /// Optional title metadata for the muxed audio track.
    pub title: Option<String>,
}

/// An attachment to embed into the output container.
#[derive(Debug, Clone)]
pub struct ProtMuxAttachment {
    /// Attachment filename stored in the container.
    pub file_name: String,
    /// MIME type stored for the attachment.
    pub mime_type: String,
    /// Raw attachment bytes.
    pub data: Vec<u8>,
}

impl ProtMuxAttachment {
    /// Create a new attachment from raw bytes.
    pub fn new(
        file_name: impl Into<String>,
        mime_type: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            file_name: file_name.into(),
            mime_type: mime_type.into(),
            data: data.into(),
        }
    }
}

/// Progress phase reported during mux creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtMuxPhase {
    /// Initial setup and validation.
    Starting,
    /// Transcoding one source path.
    Transcoding(PathBuf),
    /// Sorting encoded packets before mux writing.
    OrderingPackets,
    /// Creating the muxer and registering tracks.
    PreparingOutput,
    /// Embedding `play_settings.json`.
    EmbeddingPlaySettings,
    /// Embedding a named extra attachment.
    EmbeddingAttachment(String),
    /// Building the muxer.
    BuildingMuxer,
    /// Writing encoded packets.
    WritingPackets,
    /// Finalizing the container.
    Finalizing,
    /// Output has been completed.
    Complete(PathBuf),
}

/// Progress sink used by muxing operations.
pub trait ProtMuxProgress {
    /// Set the current high-level phase.
    fn set_phase(&mut self, phase: ProtMuxPhase);

    /// Increment completed work units.
    fn increment(&mut self, units: u64);

    /// Add work units to the total length.
    fn add_work(&mut self, units: u64);
}

/// Progress sink that ignores all events.
#[derive(Debug, Default)]
pub struct NoopProgress;

impl ProtMuxProgress for NoopProgress {
    fn set_phase(&mut self, _phase: ProtMuxPhase) {}

    fn increment(&mut self, _units: u64) {}

    fn add_work(&mut self, _units: u64) {}
}

#[derive(Debug)]
struct MuxPacket {
    track_number: u64,
    timestamp: u64,
    duration: Option<u64>,
    payload: Vec<u8>,
}

/// Create a Proteus container, discarding progress events.
///
/// # Errors
///
/// Returns [`MuxError`] if source audio cannot be decoded/encoded, mux metadata
/// is invalid, output cannot be written, or finalization fails.
pub fn create_prot(input: ProtMuxInput) -> Result<()> {
    let mut progress = NoopProgress;
    create_prot_with_progress(input, &mut progress)
}

/// Create a Proteus container and report progress to `progress`.
///
/// # Errors
///
/// Returns [`MuxError`] if source audio cannot be decoded/encoded, mux metadata
/// is invalid, output cannot be written, or finalization fails.
pub fn create_prot_with_progress(
    input: ProtMuxInput,
    progress: &mut dyn ProtMuxProgress,
) -> Result<()> {
    validate_input(&input)?;
    progress.set_phase(ProtMuxPhase::Starting);
    progress.add_work(initial_progress_units(&input));

    let mut tracks = Vec::with_capacity(input.tracks.len());
    let mut packets = Vec::new();

    for (index, track) in input.tracks.iter().enumerate() {
        progress.set_phase(ProtMuxPhase::Transcoding(track.source_path.clone()));
        let stream_serial = i32::try_from(index + 1)?;
        let encoded = transcode_to_vorbis(&track.source_path, stream_serial)?;
        let track_number = u64::try_from(index + 1)?;
        let track_uid = track_number;
        let mut config = VorbisTrackConfig::with_track_ids(
            track_number,
            track_uid,
            encoded.sample_rate,
            encoded.channels,
            encoded.headers,
        )?;

        let title = track
            .title
            .as_deref()
            .unwrap_or_else(|| path_display(&track.source_path));
        config = config.with_title(title);
        tracks.push(config);

        packets.extend(encoded.packets.into_iter().map(|packet| MuxPacket {
            track_number,
            timestamp: packet.timestamp,
            duration: packet.duration,
            payload: packet.payload,
        }));
        progress.increment(1);
    }

    progress.set_phase(ProtMuxPhase::OrderingPackets);
    packets.sort_by_key(|packet| (packet.timestamp, packet.track_number));
    progress.increment(1);
    progress.add_work(u64::try_from(packets.len())?);

    progress.set_phase(ProtMuxPhase::PreparingOutput);
    let output = File::create(&input.output)?;
    let mut builder = MkaMuxer::builder(output)
        .writing_app(input.writing_app.as_deref().unwrap_or("proteus-lib"));
    if let Some(title) = input.title {
        builder = builder.title(title);
    }
    for track in tracks {
        builder = builder.vorbis_track(track);
    }
    progress.increment(1);

    progress.set_phase(ProtMuxPhase::EmbeddingPlaySettings);
    builder = builder.attachment(to_mux_attachment(ProtMuxAttachment::new(
        "play_settings.json",
        "application/json",
        input.play_settings_json,
    ))?);
    progress.increment(1);

    for attachment in input.attachments {
        progress.set_phase(ProtMuxPhase::EmbeddingAttachment(
            attachment.file_name.clone(),
        ));
        builder = builder.attachment(to_mux_attachment(attachment)?);
        progress.increment(1);
    }

    progress.set_phase(ProtMuxPhase::BuildingMuxer);
    let mut muxer = builder.build()?;
    progress.increment(1);

    progress.set_phase(ProtMuxPhase::WritingPackets);
    for packet in packets {
        muxer.write_track_packet(
            packet.track_number,
            packet.timestamp,
            packet.duration,
            packet.payload,
        )?;
        progress.increment(1);
    }

    progress.set_phase(ProtMuxPhase::Finalizing);
    muxer.finish()?;
    progress.increment(1);
    progress.set_phase(ProtMuxPhase::Complete(input.output));
    Ok(())
}

fn validate_input(input: &ProtMuxInput) -> Result<()> {
    if input.tracks.is_empty() {
        return Err(MuxError::InvalidInput(
            "provide at least one source audio track".to_string(),
        ));
    }
    if input.play_settings_json.is_empty() {
        return Err(MuxError::InvalidInput(
            "play_settings_json must not be empty".to_string(),
        ));
    }
    for track in &input.tracks {
        if !track.source_path.is_file() {
            return Err(MuxError::InvalidInput(format!(
                "source audio file does not exist: {}",
                track.source_path.display()
            )));
        }
    }
    Ok(())
}

fn to_mux_attachment(attachment: ProtMuxAttachment) -> Result<Attachment> {
    Ok(Attachment::new(
        attachment.file_name,
        attachment.mime_type,
        attachment.data,
    )?)
}

fn initial_progress_units(input: &ProtMuxInput) -> u64 {
    u64::try_from(input.tracks.len() + input.attachments.len()).unwrap_or(0) + 5
}

fn path_display(path: &Path) -> &str {
    path.to_str().unwrap_or("audio")
}
