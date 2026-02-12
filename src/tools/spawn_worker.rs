//! Spawn worker tool for creating new workers.

use crate::{WorkerId, ChannelId};
use uuid::Uuid;

/// Create a new worker ID.
pub fn create_worker_id() -> WorkerId {
    Uuid::new_v4()
}

/// Spawn a worker for a specific task.
///
/// Workers are independent processes that execute tasks without channel context.
/// They can be fire-and-forget or interactive.
pub async fn spawn_worker(
    _channel_id: Option<ChannelId>,
    task: impl Into<String>,
    _interactive: bool,
) -> anyhow::Result<WorkerId> {
    let worker_id = create_worker_id();
    let task = task.into();
    
    tracing::info!(%worker_id, task = %task, "spawning worker");
    
    // In real implementation:
    // 1. Create a Worker process with task-specific tools (shell, file, exec)
    // 2. If interactive, set up an input channel for follow-ups
    // 3. Start the worker with its own isolated history
    // 4. Send WorkerStarted event
    
    Ok(worker_id)
}
