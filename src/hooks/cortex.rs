//! CortexHook: Prompt hook for system-level observer.

use crate::error::Result;

/// Hook for cortex observation.
#[derive(Clone)]
pub struct CortexHook;

impl CortexHook {
    /// Create a new cortex hook.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CortexHook {
    fn default() -> Self {
        Self::new()
    }
}
