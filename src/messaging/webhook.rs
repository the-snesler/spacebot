//! Webhook messaging adapter for programmatic access.

use crate::error::Result;

/// Webhook adapter state.
pub struct WebhookAdapter;

impl WebhookAdapter {
    /// Create a new webhook adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebhookAdapter {
    fn default() -> Self {
        Self::new()
    }
}
