//! `SpacebotAcpClient` — implements the ACP `Client` trait.
//!
//! Runs on a single-threaded `LocalSet`, so all internal state uses `RefCell`
//! (not `Mutex`). The `event_tx` broadcast sender is `Send` and can safely
//! emit events to the main tokio runtime.

use crate::acp::types::AcpPart;
use crate::{AgentId, ChannelId, ProcessEvent, WorkerId};

use agent_client_protocol::{
    Client, ContentBlock, PermissionOptionKind, ReadTextFileRequest, ReadTextFileResponse,
    RequestPermissionRequest, RequestPermissionResponse, RequestPermissionOutcome,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, ToolCallStatus,
    WriteTextFileRequest, WriteTextFileResponse,
};
use std::cell::RefCell;
use std::path::PathBuf;
use tokio::sync::broadcast;

/// Internal accumulated session state (single-threaded, `RefCell`-safe).
pub(super) struct AcpSessionState {
    /// Accumulated assistant text (latest chunk replaces).
    pub accumulated_text: String,
    /// Number of tool calls observed.
    pub tool_calls: i64,
    /// Accumulated parts for transcript building.
    pub parts: Vec<AcpPart>,
    /// Currently running tool name (for status updates).
    pub current_tool: Option<(String, String)>, // (id, name)
}

impl AcpSessionState {
    pub fn new() -> Self {
        Self {
            accumulated_text: String::new(),
            tool_calls: 0,
            parts: Vec::new(),
            current_tool: None,
        }
    }
}

/// ACP client implementation for Spacebot.
///
/// Handles session notifications (agent output streaming), permission requests
/// (auto-approve in headless mode), and file I/O (direct fs access).
pub struct SpacebotAcpClient {
    pub(super) worker_id: WorkerId,
    pub(super) agent_id: AgentId,
    pub(super) channel_id: Option<ChannelId>,
    pub(super) event_tx: broadcast::Sender<ProcessEvent>,
    pub(super) state: RefCell<AcpSessionState>,
    pub(super) directory: PathBuf,
}

impl SpacebotAcpClient {
    pub fn new(
        worker_id: WorkerId,
        agent_id: AgentId,
        channel_id: Option<ChannelId>,
        event_tx: broadcast::Sender<ProcessEvent>,
        directory: PathBuf,
    ) -> Self {
        Self {
            worker_id,
            agent_id,
            channel_id,
            event_tx,
            state: RefCell::new(AcpSessionState::new()),
            directory,
        }
    }

    /// Send a status update via the process event bus.
    fn send_status(&self, status: &str) {
        let _ = self.event_tx.send(ProcessEvent::WorkerStatus {
            agent_id: self.agent_id.clone(),
            worker_id: self.worker_id,
            channel_id: self.channel_id.clone(),
            status: status.to_string(),
        });
    }

    /// Emit an AcpPartUpdated event to the frontend.
    fn emit_part(&self, part: &AcpPart) {
        let _ = self.event_tx.send(ProcessEvent::AcpPartUpdated {
            agent_id: self.agent_id.clone(),
            worker_id: self.worker_id,
            channel_id: self.channel_id.clone(),
            part: part.clone(),
        });
    }

    /// Extract the accumulated result.
    pub fn take_result(&self) -> (String, Vec<AcpPart>, i64) {
        let state = self.state.borrow();
        (
            state.accumulated_text.clone(),
            state.parts.clone(),
            state.tool_calls,
        )
    }
}

#[async_trait::async_trait(?Send)]
impl Client for SpacebotAcpClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> agent_client_protocol::Result<RequestPermissionResponse> {
        // Emit a permission event for observability.
        let description = args
            .tool_call
            .fields
            .title
            .as_deref()
            .unwrap_or("unknown operation");
        let _ = self.event_tx.send(ProcessEvent::WorkerPermission {
            agent_id: self.agent_id.clone(),
            worker_id: self.worker_id,
            channel_id: self.channel_id.clone(),
            permission_id: args.tool_call.tool_call_id.0.to_string(),
            description: description.to_string(),
            patterns: Vec::new(),
        });

        // Auto-approve: find the first "allow_once" or "allow_always" option.
        let option_id = args
            .options
            .iter()
            .find(|o| {
                matches!(
                    o.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                )
            })
            .map(|o| o.option_id.clone())
            .unwrap_or_else(|| {
                // Fallback: first option.
                args.options
                    .first()
                    .map(|o| o.option_id.clone())
                    .unwrap_or_else(|| agent_client_protocol::PermissionOptionId::new("allow"))
            });

        Ok(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        ))
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        let mut state = self.state.borrow_mut();

        match args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                // Extract text from the content block.
                if let ContentBlock::Text(text_content) = chunk.content {
                    state.accumulated_text.push_str(&text_content.text);
                    let part = AcpPart::Text {
                        text: text_content.text,
                    };
                    state.parts.push(part.clone());
                    drop(state);
                    self.emit_part(&part);
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let ContentBlock::Text(text_content) = chunk.content {
                    let part = AcpPart::Thought {
                        text: text_content.text,
                    };
                    state.parts.push(part.clone());
                    drop(state);
                    self.emit_part(&part);
                }
            }
            SessionUpdate::ToolCall(tool_call) => {
                state.tool_calls += 1;
                let id = tool_call.tool_call_id.0.to_string();
                let name = tool_call.title.clone();

                let part = AcpPart::ToolStarted {
                    id: id.clone(),
                    name: name.clone(),
                };
                state.parts.push(part.clone());
                state.current_tool = Some((id, name.clone()));
                drop(state);

                self.emit_part(&part);
                self.send_status(&format!("running: {name}"));
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let id = update.tool_call_id.0.to_string();
                let is_completed = update
                    .fields
                    .status
                    .is_some_and(|s| matches!(s, ToolCallStatus::Completed | ToolCallStatus::Failed));

                if is_completed {
                    let name = update
                        .fields
                        .title
                        .clone()
                        .or_else(|| {
                            state
                                .current_tool
                                .as_ref()
                                .filter(|(tid, _)| tid == &id)
                                .map(|(_, n)| n.clone())
                        })
                        .unwrap_or_else(|| "tool".to_string());

                    // Extract result text from raw_output or content.
                    let result = update
                        .fields
                        .raw_output
                        .as_ref()
                        .map(|v| {
                            let s = v.to_string();
                            if s.len() > 2_000 {
                                crate::tools::truncate_output(&s, 2_000)
                            } else {
                                s
                            }
                        })
                        .unwrap_or_default();

                    let part = AcpPart::ToolCompleted {
                        id: id.clone(),
                        name: name.clone(),
                        result,
                    };
                    state.parts.push(part.clone());
                    if state
                        .current_tool
                        .as_ref()
                        .is_some_and(|(tid, _)| tid == &id)
                    {
                        state.current_tool = None;
                    }
                    drop(state);

                    self.emit_part(&part);
                    self.send_status(&format!("done: {name}"));
                }
            }
            _ => {
                // Plan, AvailableCommandsUpdate, CurrentModeUpdate, etc. — log and ignore.
                tracing::trace!(worker_id = %self.worker_id, "ignoring ACP session update");
            }
        }

        Ok(())
    }

    async fn read_text_file(
        &self,
        args: ReadTextFileRequest,
    ) -> agent_client_protocol::Result<ReadTextFileResponse> {
        let path = args.path;
        // Resolve relative paths against the working directory.
        let resolved = if path.is_absolute() {
            path
        } else {
            self.directory.join(path)
        };

        match std::fs::read_to_string(&resolved) {
            Ok(content) => Ok(ReadTextFileResponse::new(content)),
            Err(error) => {
                tracing::warn!(
                    worker_id = %self.worker_id,
                    path = %resolved.display(),
                    %error,
                    "ACP read_text_file failed"
                );
                Err(agent_client_protocol::Error::internal_error()
                    .data(serde_json::json!(error.to_string())))
            }
        }
    }

    async fn write_text_file(
        &self,
        args: WriteTextFileRequest,
    ) -> agent_client_protocol::Result<WriteTextFileResponse> {
        let path = args.path;
        let resolved = if path.is_absolute() {
            path
        } else {
            self.directory.join(path)
        };

        // Ensure parent directory exists.
        if let Some(parent) = resolved.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    worker_id = %self.worker_id,
                    path = %resolved.display(),
                    %error,
                    "ACP write_text_file: failed to create parent directory"
                );
                return Err(agent_client_protocol::Error::internal_error()
                    .data(serde_json::json!(error.to_string())));
            }
        }

        match std::fs::write(&resolved, &args.content) {
            Ok(()) => Ok(WriteTextFileResponse::new()),
            Err(error) => {
                tracing::warn!(
                    worker_id = %self.worker_id,
                    path = %resolved.display(),
                    %error,
                    "ACP write_text_file failed"
                );
                Err(agent_client_protocol::Error::internal_error()
                    .data(serde_json::json!(error.to_string())))
            }
        }
    }
}
