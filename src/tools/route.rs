//! Route tool for sending follow-ups to active workers.

use crate::{WorkerId, ChannelId};

/// Route a message to an active interactive worker.
///
/// This is how the channel continues a conversation with a long-running
/// worker (like a coding session) without creating a new branch.
pub async fn route_to_worker(
    _channel_id: ChannelId,
    _worker_id: WorkerId,
    _message: impl Into<String>,
) -> anyhow::Result<()> {
    // In real implementation:
    // 1. Verify worker exists and is interactive
    // 2. Queue the message on the worker's input channel
    // 3. Return immediately (don't wait for response)
    
    Ok(())
}
