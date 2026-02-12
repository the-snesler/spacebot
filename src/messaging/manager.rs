//! MessagingManager: Fan-in and routing for all adapters.

use crate::error::Result;
use crate::messaging::traits::{Messaging, MessagingDyn};
use std::collections::HashMap;
use std::sync::Arc;

/// Manages all messaging adapters.
pub struct MessagingManager {
    adapters: HashMap<String, Arc<dyn MessagingDyn>>,
}

impl MessagingManager {
    /// Create a new messaging manager.
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register an adapter.
    pub fn register(&mut self, adapter: impl Messaging) {
        let name = adapter.name().to_string();
        self.adapters.insert(name, Arc::new(adapter));
    }
}

impl Default for MessagingManager {
    fn default() -> Self {
        Self::new()
    }
}
