//! Cancel tool for stopping workers or branches.

use crate::{WorkerId, BranchId, ChannelId};

/// Cancel an active worker.
pub async fn cancel_worker(
    _channel_id: ChannelId,
    _worker_id: WorkerId,
) -> anyhow::Result<()> {
    // In real implementation:
    // 1. Send cancellation signal to worker
    // 2. Remove from active_workers list
    // 3. Send WorkerComplete event with cancelled status
    
    Ok(())
}

/// Cancel an active branch.
pub async fn cancel_branch(
    _channel_id: ChannelId,
    _branch_id: BranchId,
) -> anyhow::Result<()> {
    // In real implementation:
    // 1. Send cancellation signal to branch
    // 2. Remove from active_branches list
    // 3. Do NOT send BranchResult (branch is aborted, not completed)
    
    Ok(())
}
