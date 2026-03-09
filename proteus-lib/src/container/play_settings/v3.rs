//! `play_settings.json` version 3 schema.

use super::{PlaySettingsPayload, VersionedPlaySettingsFile};

/// Version 3 settings payload.
pub type PlaySettingsV3 = PlaySettingsPayload;
/// Top-level wrapper for V3 settings files.
pub type PlaySettingsV3File = VersionedPlaySettingsFile<PlaySettingsV3>;
