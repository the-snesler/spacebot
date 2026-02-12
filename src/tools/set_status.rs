//! Set status tool for workers.

use crate::{ProcessEvent, WorkerId};
use tokio::sync::mpsc;

/// Set the status of a worker.
pub fn set_status(
    worker_id: WorkerId,
    status: impl Into<String>,
    event_tx: &mpsc::Sender<ProcessEvent>,
) {
    let event = ProcessEvent::WorkerStatus {
        worker_id,
        channel_id: None, // Will be filled in by caller
        status: status.into(),
    };

    // Send without blocking - if channel is full, that's ok
    let _ = event_tx.try_send(event);
}
