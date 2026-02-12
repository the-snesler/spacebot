//! Telegram messaging adapter.

use crate::error::Result;

/// Telegram adapter state.
pub struct TelegramAdapter;

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TelegramAdapter {
    fn default() -> Self {
        Self::new()
    }
}
