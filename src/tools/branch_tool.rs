//! Branch tool for forking context and thinking (channel only).

use crate::{BranchId, ChannelId};
use uuid::Uuid;

/// Create a new branch ID.
pub fn create_branch_id() -> BranchId {
    Uuid::new_v4()
}

/// Spawn a branch for thinking.
/// 
/// This creates a branch process that has a clone of the channel's
/// context and can use tools like memory_recall and spawn_worker.
/// The branch returns a conclusion that gets incorporated into the channel.
pub async fn spawn_branch(
    _channel_id: ChannelId,
    _description: impl Into<String>,
) -> anyhow::Result<BranchId> {
    let branch_id = create_branch_id();
    
    tracing::info!(%branch_id, "spawning branch");
    
    // In real implementation, this would:
    // 1. Clone the channel's history
    // 2. Create a new Branch process
    // 3. Add to channel's active_branches
    // 4. Return the branch_id for tracking
    
    Ok(branch_id)
}
