//! Discord messaging adapter.

use crate::error::Result;

/// Discord adapter state.
pub struct DiscordAdapter;

impl DiscordAdapter {
    /// Create a new Discord adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiscordAdapter {
    fn default() -> Self {
        Self::new()
    }
}
