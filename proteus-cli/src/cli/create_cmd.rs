//! Create command handlers.

use log::error;

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
