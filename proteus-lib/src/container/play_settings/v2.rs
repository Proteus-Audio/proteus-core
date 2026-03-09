//! `play_settings.json` version 2 schema.

use super::{PlaySettingsPayload, VersionedPlaySettingsFile};

/// Version 2 settings payload.
pub type PlaySettingsV2 = PlaySettingsPayload;
/// Top-level wrapper for V2 settings files.
pub type PlaySettingsV2File = VersionedPlaySettingsFile<PlaySettingsV2>;
