//! Key-value settings storage (redb).

use crate::error::Result;

/// Settings store.
pub struct SettingsStore;

impl SettingsStore {
    /// Create a new settings store.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self::new()
    }
}
