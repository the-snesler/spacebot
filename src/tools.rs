//! Tools available to agents.

pub mod reply;
pub mod branch_tool;
pub mod spawn_worker;
pub mod route;
pub mod cancel;
pub mod memory_save;
pub mod memory_recall;
pub mod set_status;
pub mod shell;
pub mod file;
pub mod exec;

use rig::tool::{Tool as RigTool, ToolSet};

/// Tool server handle for sharing tools across agents.
/// Wraps Rig's ToolSet.
pub struct ToolServerHandle {
    tool_set: ToolSet,
}

impl ToolServerHandle {
    /// Create a new tool server handle.
    pub fn new() -> Self {
        Self {
            tool_set: ToolSet::default(),
        }
    }
    
    /// Register a Rig-compatible tool.
    pub fn register(&mut self, tool: impl RigTool + 'static) {
        self.tool_set.add_tool(tool);
    }
    
    /// Get the inner Rig ToolSet.
    pub fn tool_set(&self) -> &ToolSet {
        &self.tool_set
    }
}

impl Default for ToolServerHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ToolServerHandle {
    fn clone(&self) -> Self {
        // ToolSet doesn't implement Clone, so we create a new empty one
        // Tools should be registered after cloning
        Self::new()
    }
}
