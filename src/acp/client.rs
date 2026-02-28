//! ACP Client trait implementation for Spacebot.
//!
//! Implements the `agent_client_protocol::Client` trait, providing file I/O
//! (with sandbox for terminal operations), permission auto-approval with audit
//! events, and session notification handling that maps to `ProcessEvent` status
//! updates.

use crate::sandbox::Sandbox;
use crate::{AgentId, ChannelId, ProcessEvent, WorkerId};

use agent_client_protocol::{
    Client, ContentBlock, CreateTerminalRequest, CreateTerminalResponse,
    KillTerminalCommandRequest, KillTerminalCommandResponse, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, TerminalExitStatus, TerminalId,
    TerminalOutputRequest, TerminalOutputResponse, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Maximum bytes of output captured per terminal.
const MAX_TERMINAL_OUTPUT_BYTES: usize = 64 * 1024;

/// Spacebot's ACP client implementation.
///
/// Handles agent callbacks: file I/O, terminal management, permission requests,
/// and session notifications. Uses `RefCell` for interior mutability because
/// the ACP crate's `Client` trait is `!Send` (single-threaded async via `LocalSet`).
pub struct SpacebotAcpClient {
    working_dir: PathBuf,
    sandbox: Arc<Sandbox>,
    event_tx: broadcast::Sender<ProcessEvent>,
    agent_id: AgentId,
    worker_id: WorkerId,
    channel_id: Option<ChannelId>,
    /// Active terminals spawned by the agent.
    terminals: RefCell<HashMap<String, TerminalState>>,
    terminal_counter: RefCell<u64>,
    /// Accumulated agent message text (the final result).
    /// Uses `Arc<std::sync::Mutex>` so the worker can read the result after
    /// the client is moved into the connection.
    result_text: Arc<std::sync::Mutex<String>>,
}

/// State for a terminal spawned by the ACP agent.
struct TerminalState {
    child: Child,
    /// Captured stdout+stderr output (capped at [`MAX_TERMINAL_OUTPUT_BYTES`]).
    output: Arc<std::sync::Mutex<Vec<u8>>>,
    /// Background task reading the process output.
    _output_task: JoinHandle<()>,
}

impl SpacebotAcpClient {
    pub fn new(
        working_dir: PathBuf,
        sandbox: Arc<Sandbox>,
        event_tx: broadcast::Sender<ProcessEvent>,
        agent_id: AgentId,
        worker_id: WorkerId,
        channel_id: Option<ChannelId>,
        result_text: Arc<std::sync::Mutex<String>>,
    ) -> Self {
        Self {
            working_dir,
            sandbox,
            event_tx,
            agent_id,
            worker_id,
            channel_id,
            terminals: RefCell::new(HashMap::new()),
            terminal_counter: RefCell::new(0),
            result_text,
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

    /// Generate a unique terminal ID.
    fn next_terminal_id(&self) -> String {
        let mut counter = self.terminal_counter.borrow_mut();
        *counter += 1;
        format!("term-{}", *counter)
    }

    /// Resolve a path from the agent relative to the working directory.
    /// Prevents path traversal outside the working directory.
    fn resolve_path(&self, path: &Path) -> agent_client_protocol::Result<PathBuf> {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.working_dir.join(path)
        };

        // Canonicalize what exists, or check prefix for new files
        let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());

        let working_canonical = self
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| self.working_dir.clone());

        if !canonical.starts_with(&working_canonical) {
            return Err(agent_client_protocol::Error::new(
                -32002,
                format!("Path '{}' is outside the working directory", path.display()),
            ));
        }

        Ok(resolved)
    }
}

#[async_trait::async_trait(?Send)]
impl Client for SpacebotAcpClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> agent_client_protocol::Result<RequestPermissionResponse> {
        let tool_title = args.tool_call.fields.title.as_deref().unwrap_or("unknown");
        tracing::info!(
            worker_id = %self.worker_id,
            tool = %tool_title,
            options_count = args.options.len(),
            "ACP agent requesting permission"
        );

        // Emit audit event
        let description = format!("ACP permission: {tool_title}");
        let _ = self.event_tx.send(ProcessEvent::WorkerPermission {
            agent_id: self.agent_id.clone(),
            worker_id: self.worker_id,
            channel_id: self.channel_id.clone(),
            permission_id: args.tool_call.tool_call_id.to_string(),
            description,
            patterns: vec![tool_title.to_string()],
        });

        // Auto-approve: pick the first AllowOnce or AllowAlways option, or the first option
        let selected_id = args
            .options
            .iter()
            .find(|o| {
                matches!(
                    o.kind,
                    agent_client_protocol::PermissionOptionKind::AllowOnce
                        | agent_client_protocol::PermissionOptionKind::AllowAlways
                )
            })
            .or_else(|| args.options.first())
            .map(|o| o.option_id.clone());

        if let Some(option_id) = selected_id {
            Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
            ))
        } else {
            // No options — cancel
            Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ))
        }
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        match args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(text_content) = chunk.content
                    && let Ok(mut result) = self.result_text.lock()
                {
                    result.push_str(&text_content.text);
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let ContentBlock::Text(text_content) = chunk.content {
                    tracing::trace!(
                        worker_id = %self.worker_id,
                        thought_len = text_content.text.len(),
                        "ACP agent thought"
                    );
                }
            }
            SessionUpdate::ToolCall(tool_call) => {
                self.send_status(&format!("running: {}", tool_call.title));
            }
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some(status) = &update.fields.status {
                    match status {
                        agent_client_protocol::ToolCallStatus::Completed => {
                            self.send_status("working");
                        }
                        agent_client_protocol::ToolCallStatus::Failed => {
                            let title = update.fields.title.as_deref().unwrap_or("unknown");
                            self.send_status(&format!("tool error: {title}"));
                        }
                        _ => {}
                    }
                }
            }
            SessionUpdate::Plan(_plan) => {
                self.send_status("planning");
            }
            _ => {}
        }
        Ok(())
    }

    async fn read_text_file(
        &self,
        args: ReadTextFileRequest,
    ) -> agent_client_protocol::Result<ReadTextFileResponse> {
        let path = self.resolve_path(&args.path)?;

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            agent_client_protocol::Error::resource_not_found(Some(path.display().to_string()))
                .data(serde_json::json!({ "error": e.to_string() }))
        })?;

        // Apply line offset and limit if specified
        let content = match (args.line, args.limit) {
            (Some(line), Some(limit)) => {
                let start = (line as usize).saturating_sub(1);
                content
                    .lines()
                    .skip(start)
                    .take(limit as usize)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            (Some(line), None) => {
                let start = (line as usize).saturating_sub(1);
                content.lines().skip(start).collect::<Vec<_>>().join("\n")
            }
            (None, Some(limit)) => content
                .lines()
                .take(limit as usize)
                .collect::<Vec<_>>()
                .join("\n"),
            (None, None) => content,
        };

        Ok(ReadTextFileResponse::new(content))
    }

    async fn write_text_file(
        &self,
        args: WriteTextFileRequest,
    ) -> agent_client_protocol::Result<WriteTextFileResponse> {
        let path = self.resolve_path(&args.path)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                agent_client_protocol::Error::internal_error()
                    .data(serde_json::json!({ "error": e.to_string() }))
            })?;
        }

        tokio::fs::write(&path, &args.content).await.map_err(|e| {
            agent_client_protocol::Error::internal_error()
                .data(serde_json::json!({ "error": e.to_string() }))
        })?;

        Ok(WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        args: CreateTerminalRequest,
    ) -> agent_client_protocol::Result<CreateTerminalResponse> {
        let cwd = args.cwd.as_deref().unwrap_or(&self.working_dir);

        // Build args for sandbox.wrap()
        let mut shell_args: Vec<String> = vec![args.command.clone()];
        shell_args.extend(args.args.iter().cloned());
        let shell_cmd = shell_args.join(" ");

        let mut cmd = self.sandbox.wrap("sh", &["-c", &shell_cmd], cwd);

        // Inject environment variables from the request
        for env_var in &args.env {
            cmd.env(&env_var.name, &env_var.value);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            agent_client_protocol::Error::internal_error()
                .data(serde_json::json!({ "error": e.to_string() }))
        })?;

        let terminal_id = self.next_terminal_id();
        let output_buf: Arc<std::sync::Mutex<Vec<u8>>> =
            Arc::new(std::sync::Mutex::new(Vec::with_capacity(4096)));

        // Merge stdout and stderr into the output buffer
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let output_clone = output_buf.clone();
        let output_byte_limit = args
            .output_byte_limit
            .map(|l| l as usize)
            .unwrap_or(MAX_TERMINAL_OUTPUT_BYTES);

        let output_task = tokio::spawn(async move {
            let mut stdout_buf = [0u8; 4096];
            let mut stderr_buf = [0u8; 4096];

            let mut stdout = stdout.map(tokio::io::BufReader::new);
            let mut stderr = stderr.map(tokio::io::BufReader::new);

            loop {
                tokio::select! {
                    result = async {
                        match stdout.as_mut() {
                            Some(s) => s.read(&mut stdout_buf).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match result {
                            Ok(0) => { stdout = None; }
                            Ok(n) => {
                                let mut buf = output_clone.lock().unwrap();
                                let remaining = output_byte_limit.saturating_sub(buf.len());
                                buf.extend_from_slice(&stdout_buf[..n.min(remaining)]);
                            }
                            Err(_) => { stdout = None; }
                        }
                    }
                    result = async {
                        match stderr.as_mut() {
                            Some(s) => s.read(&mut stderr_buf).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match result {
                            Ok(0) => { stderr = None; }
                            Ok(n) => {
                                let mut buf = output_clone.lock().unwrap();
                                let remaining = output_byte_limit.saturating_sub(buf.len());
                                buf.extend_from_slice(&stderr_buf[..n.min(remaining)]);
                            }
                            Err(_) => { stderr = None; }
                        }
                    }
                }

                if stdout.is_none() && stderr.is_none() {
                    break;
                }
            }
        });

        self.terminals.borrow_mut().insert(
            terminal_id.clone(),
            TerminalState {
                child,
                output: output_buf,
                _output_task: output_task,
            },
        );

        Ok(CreateTerminalResponse::new(TerminalId::new(terminal_id)))
    }

    async fn terminal_output(
        &self,
        args: TerminalOutputRequest,
    ) -> agent_client_protocol::Result<TerminalOutputResponse> {
        let terminal_id = args.terminal_id.to_string();
        let terminals = self.terminals.borrow();
        let terminal = terminals.get(&terminal_id).ok_or_else(|| {
            agent_client_protocol::Error::resource_not_found(Some(terminal_id.clone()))
        })?;

        let buf = terminal.output.lock().unwrap();
        let output = String::from_utf8_lossy(&buf).into_owned();
        let truncated = buf.len() >= MAX_TERMINAL_OUTPUT_BYTES;

        let exit_status = terminal
            .child
            .id()
            .is_none()
            .then(TerminalExitStatus::new);

        drop(buf);

        let mut response = TerminalOutputResponse::new(output, truncated);
        if let Some(status) = exit_status {
            response = response.exit_status(status);
        }

        Ok(response)
    }

    async fn wait_for_terminal_exit(
        &self,
        args: WaitForTerminalExitRequest,
    ) -> agent_client_protocol::Result<WaitForTerminalExitResponse> {
        let terminal_id = args.terminal_id.to_string();

        // Take the terminal out of the map so we can await without holding the RefCell borrow.
        let mut terminal = self
            .terminals
            .borrow_mut()
            .remove(&terminal_id)
            .ok_or_else(|| {
                agent_client_protocol::Error::resource_not_found(Some(terminal_id.clone()))
            })?;

        let wait_result = terminal.child.wait().await;

        // Put it back after awaiting.
        self.terminals
            .borrow_mut()
            .insert(terminal_id.clone(), terminal);

        let exit_status = match wait_result {
            Ok(status) => {
                let mut es = TerminalExitStatus::new();
                if let Some(code) = status.code() {
                    es = es.exit_code(code as u32);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        es = es.signal(signal.to_string());
                    }
                }
                es
            }
            Err(e) => {
                tracing::warn!(
                    terminal_id = %terminal_id,
                    error = %e,
                    "Failed to wait for terminal exit"
                );
                TerminalExitStatus::new()
            }
        };

        Ok(WaitForTerminalExitResponse::new(exit_status))
    }

    async fn kill_terminal_command(
        &self,
        args: KillTerminalCommandRequest,
    ) -> agent_client_protocol::Result<KillTerminalCommandResponse> {
        let terminal_id = args.terminal_id.to_string();
        let mut taken = self.terminals.borrow_mut().remove(&terminal_id);
        if let Some(ref mut terminal) = taken {
            let _ = terminal.child.kill().await;
        }
        // Put it back if it existed.
        if let Some(terminal) = taken {
            self.terminals.borrow_mut().insert(terminal_id, terminal);
        }
        Ok(KillTerminalCommandResponse::new())
    }

    async fn release_terminal(
        &self,
        args: ReleaseTerminalRequest,
    ) -> agent_client_protocol::Result<ReleaseTerminalResponse> {
        let terminal_id = args.terminal_id.to_string();
        let taken = self.terminals.borrow_mut().remove(&terminal_id);
        if let Some(mut terminal) = taken {
            let _ = terminal.child.kill().await;
            terminal._output_task.abort();
        }
        Ok(ReleaseTerminalResponse::new())
    }
}
