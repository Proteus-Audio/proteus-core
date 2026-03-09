//! `play_settings.json` version 1 schema.

use super::{PlaySettingsPayload, VersionedPlaySettingsFile};

/// Version 1 settings payload.
pub type PlaySettingsV1 = PlaySettingsPayload;
/// Top-level wrapper for V1 settings files.
pub type PlaySettingsV1File = VersionedPlaySettingsFile<PlaySettingsV1>;
