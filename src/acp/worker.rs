//! ACP worker: drives an ACP agent session for coding tasks.
//!
//! Spawns an external coding agent subprocess, establishes a JSON-RPC connection
//! over stdin/stdout using the `agent-client-protocol` crate, and manages the
//! full session lifecycle including initialization, prompting, streaming
//! notifications, interactive follow-ups, timeout enforcement, and graceful
//! cancellation.

use crate::acp::client::SpacebotAcpClient;
use crate::acp::process::AcpProcess;
use crate::config::AcpAgentConfig;
use crate::sandbox::Sandbox;
use crate::{AgentId, ChannelId, ProcessEvent, WorkerId};

use agent_client_protocol::{
    Agent as _, CancelNotification, ClientCapabilities, ClientSideConnection, ContentBlock,
    FileSystemCapability, Implementation, InitializeRequest, NewSessionRequest, PromptRequest,
    ProtocolVersion, TextContent,
};
use anyhow::{Context as _, bail};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Custom error type to distinguish cancellation from real failures.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Worker cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// An ACP-backed worker that drives a coding session via subprocess.
pub struct AcpWorker {
    pub id: WorkerId,
    pub channel_id: Option<ChannelId>,
    pub agent_id: AgentId,
    pub task: String,
    pub directory: PathBuf,
    pub acp_config: AcpAgentConfig,
    pub sandbox: Arc<Sandbox>,
    pub event_tx: broadcast::Sender<ProcessEvent>,
    pub input_rx: Option<mpsc::Receiver<String>>,
    pub cancellation_token: CancellationToken,
}

/// Result of an ACP worker run.
pub struct AcpWorkerResult {
    pub session_id: String,
    pub result_text: String,
}

impl AcpWorker {
    /// Create a new ACP worker.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        channel_id: Option<ChannelId>,
        agent_id: AgentId,
        task: impl Into<String>,
        directory: PathBuf,
        acp_config: AcpAgentConfig,
        sandbox: Arc<Sandbox>,
        event_tx: broadcast::Sender<ProcessEvent>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel_id,
            agent_id,
            task: task.into(),
            directory,
            acp_config,
            sandbox,
            event_tx,
            input_rx: None,
            cancellation_token,
        }
    }

    /// Create an interactive ACP worker that accepts follow-up messages.
    #[allow(clippy::too_many_arguments)]
    pub fn new_interactive(
        channel_id: Option<ChannelId>,
        agent_id: AgentId,
        task: impl Into<String>,
        directory: PathBuf,
        acp_config: AcpAgentConfig,
        sandbox: Arc<Sandbox>,
        event_tx: broadcast::Sender<ProcessEvent>,
        cancellation_token: CancellationToken,
    ) -> (Self, mpsc::Sender<String>) {
        let (input_tx, input_rx) = mpsc::channel(32);
        let mut worker = Self::new(
            channel_id,
            agent_id,
            task,
            directory,
            acp_config,
            sandbox,
            event_tx,
            cancellation_token,
        );
        worker.input_rx = Some(input_rx);
        (worker, input_tx)
    }

    /// Run the ACP worker lifecycle.
    ///
    /// Spawns the subprocess, establishes the JSON-RPC connection, initializes
    /// the session, sends the task as a prompt, processes streaming notifications,
    /// handles interactive follow-ups, and enforces timeout/cancellation.
    pub async fn run(mut self) -> anyhow::Result<AcpWorkerResult> {
        self.ensure_not_cancelled()?;
        self.send_status("starting ACP agent");

        // Spawn the ACP subprocess
        let mut process = AcpProcess::spawn(&self.acp_config, &self.directory)
            .context("failed to spawn ACP agent process")?;

        let stdin = process
            .stdin
            .take()
            .context("ACP process stdin not available")?;
        let stdout = process
            .stdout
            .take()
            .context("ACP process stdout not available")?;

        // Run the ACP protocol session on a LocalSet (the crate is !Send)
        let timeout_secs = self.acp_config.timeout;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            self.run_session(stdin, stdout),
        )
        .await;

        match result {
            Ok(Ok(worker_result)) => {
                // Clean shutdown
                process.kill().await;
                self.send_status("completed");
                Ok(worker_result)
            }
            Ok(Err(err)) => {
                if err.downcast_ref::<Cancelled>().is_some() {
                    self.send_status("cancelled");
                    process.kill().await;
                    return Err(err);
                }

                self.send_status("failed");
                // Session error — collect stderr for diagnostics
                let stderr = process.stderr_output().await;
                process.kill().await;
                let context = if stderr.is_empty() {
                    String::new()
                } else {
                    let truncated = if stderr.len() > 2000 {
                        &stderr[stderr.len() - 2000..]
                    } else {
                        &stderr
                    };
                    format!("\nAgent stderr:\n{truncated}")
                };
                bail!("ACP worker failed: {err}{context}")
            }
            Err(_timeout) => {
                // Timeout — try to cancel gracefully, then kill
                tracing::warn!(
                    worker_id = %self.id,
                    timeout_secs,
                    "ACP worker timed out"
                );
                self.send_status("timed out");
                self.cancellation_token.cancel();
                process.kill().await;
                bail!("ACP worker timed out after {timeout_secs}s")
            }
        }
    }

    /// Run the ACP protocol session (initialize, prompt, process notifications).
    ///
    /// The `agent-client-protocol` crate's `Client` trait is `!Send`, so the
    /// session must run on a `LocalSet`. Run that `LocalSet` on a dedicated
    /// OS thread and deliver the result back via a oneshot channel so long-lived
    /// ACP sessions do not occupy Tokio's blocking thread pool.
    async fn run_session(
        &mut self,
        stdin: tokio::process::ChildStdin,
        stdout: tokio::process::ChildStdout,
    ) -> anyhow::Result<AcpWorkerResult> {
        let worker_id = self.id;
        let agent_id = self.agent_id.clone();
        let channel_id = self.channel_id.clone();
        let event_tx = self.event_tx.clone();
        let sandbox = self.sandbox.clone();
        let working_dir = self.directory.clone();
        let task = self.task.clone();
        let cancellation_token = self.cancellation_token.clone();
        let input_rx = self.input_rx.take();

        // Shared buffer for accumulating result text from session notifications
        let result_text = Arc::new(std::sync::Mutex::new(String::new()));
        let result_text_clone = result_text.clone();

        // Capture the current runtime handle so we can re-enter it from the
        // dedicated session thread. This lets us use the same I/O driver for the piped
        // stdin/stdout while running !Send futures on a LocalSet.
        let rt_handle = tokio::runtime::Handle::current();
        let (result_tx, result_rx) = oneshot::channel();

        std::thread::Builder::new()
            .name(format!("acp-session-{worker_id}"))
            .spawn(move || {
                let result = rt_handle.block_on(async move {
                    let local = tokio::task::LocalSet::new();
                    local
                        .run_until(async move {
                            // Create the ACP client
                            let client = SpacebotAcpClient::new(
                                working_dir.clone(),
                                sandbox,
                                event_tx.clone(),
                                agent_id.clone(),
                                worker_id,
                                channel_id.clone(),
                                result_text_clone,
                            );

                            // Wrap stdin/stdout for the async I/O the crate expects
                            let async_stdin =
                                tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(stdin);
                            let async_stdout =
                                tokio_util::compat::TokioAsyncReadCompatExt::compat(stdout);

                            let (connection, io_task) =
                                ClientSideConnection::new(client, async_stdin, async_stdout, |fut| {
                                    tokio::task::spawn_local(fut);
                                });

                            // Spawn the I/O handler
                            let io_handle = tokio::task::spawn_local(async move {
                                if let Err(e) = io_task.await {
                                    tracing::trace!(error = %e, "ACP I/O task ended");
                                }
                            });

                            // Initialize the connection
                            let _init_response = connection
                                .initialize(
                                    InitializeRequest::new(ProtocolVersion::V1)
                                        .client_capabilities(
                                            ClientCapabilities::new()
                                                .fs(FileSystemCapability::new()
                                                    .read_text_file(true)
                                                    .write_text_file(true))
                                                .terminal(true),
                                        )
                                        .client_info(Implementation::new(
                                            "spacebot",
                                            env!("CARGO_PKG_VERSION"),
                                        )),
                                )
                                .await
                                .context("ACP initialization failed")?;

                            // Create a new session
                            let session_response = connection
                                .new_session(NewSessionRequest::new(&working_dir))
                                .await
                                .context("ACP session creation failed")?;

                            let session_id = session_response.session_id;

                            tracing::info!(
                                worker_id = %worker_id,
                                session_id = %session_id,
                                "ACP session created"
                            );

                            let _ = event_tx.send(ProcessEvent::WorkerStatus {
                                agent_id: agent_id.clone(),
                                worker_id,
                                channel_id: channel_id.clone(),
                                status: "sending task".to_string(),
                            });

                            // Check cancellation before sending prompt
                            if cancellation_token.is_cancelled() {
                                let _ = connection
                                    .cancel(CancelNotification::new(session_id.clone()))
                                    .await;
                                bail!(Cancelled);
                            }

                            // Send the initial prompt
                            let prompt_response = connection
                                .prompt(PromptRequest::new(
                                    session_id.clone(),
                                    vec![ContentBlock::Text(TextContent::new(&task))],
                                ))
                                .await
                                .context("ACP prompt failed")?;

                            tracing::info!(
                                worker_id = %worker_id,
                                stop_reason = ?prompt_response.stop_reason,
                                "ACP initial prompt completed"
                            );

                            // Interactive follow-up loop
                            if let Some(mut input_rx) = input_rx {
                                let _ = event_tx.send(ProcessEvent::WorkerStatus {
                                    agent_id: agent_id.clone(),
                                    worker_id,
                                    channel_id: channel_id.clone(),
                                    status: "waiting for follow-up".to_string(),
                                });

                                loop {
                                    tokio::select! {
                                        follow_up = input_rx.recv() => {
                                            let Some(follow_up) = follow_up else {
                                                break; // Channel closed
                                            };

                                            if cancellation_token.is_cancelled() {
                                                let _ = connection.cancel(
                                                    CancelNotification::new(session_id.clone()),
                                                ).await;
                                                break;
                                            }

                                            let _ = event_tx.send(ProcessEvent::WorkerStatus {
                                                agent_id: agent_id.clone(),
                                                worker_id,
                                                channel_id: channel_id.clone(),
                                                status: "processing follow-up".to_string(),
                                            });

                                            let previous_result_len = result_text
                                                .lock()
                                                .map(|text| text.len())
                                                .unwrap_or(0);

                                            match connection.prompt(PromptRequest::new(
                                                session_id.clone(),
                                                vec![ContentBlock::Text(TextContent::new(&follow_up))],
                                            )).await {
                                                Ok(_) => {
                                                    let follow_up_result = result_text
                                                        .lock()
                                                        .ok()
                                                        .and_then(|text| {
                                                            text.get(previous_result_len..)
                                                                .map(|slice| slice.to_string())
                                                        })
                                                        .unwrap_or_default();

                                                    if !follow_up_result.trim().is_empty() {
                                                        let _ = event_tx.send(ProcessEvent::WorkerResult {
                                                            agent_id: agent_id.clone(),
                                                            worker_id,
                                                            channel_id: channel_id.clone(),
                                                            result: follow_up_result,
                                                        });
                                                    }

                                                    let _ = event_tx.send(ProcessEvent::WorkerStatus {
                                                        agent_id: agent_id.clone(),
                                                        worker_id,
                                                        channel_id: channel_id.clone(),
                                                        status: "waiting for follow-up".to_string(),
                                                    });
                                                }
                                                Err(error) => {
                                                    tracing::error!(
                                                        worker_id = %worker_id,
                                                        %error,
                                                        "ACP follow-up failed"
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                        _ = cancellation_token.cancelled() => {
                                            let _ = connection.cancel(
                                                CancelNotification::new(session_id.clone()),
                                            ).await;
                                            break;
                                        }
                                    }
                                }
                            }

                            // Result text was accumulated by the client's
                            // session_notification handler via the shared
                            // Arc<Mutex<String>>.
                            let result_text = result_text.lock().map(|s| s.clone()).unwrap_or_default();

                            // Clean up
                            io_handle.abort();

                            Ok(AcpWorkerResult {
                                session_id: session_id.to_string(),
                                result_text,
                            })
                        })
                        .await
                });
                let _ = result_tx.send(result);
            })
            .context("failed to spawn ACP session thread")?;

        result_rx
            .await
            .map_err(|_| anyhow::anyhow!("ACP session thread terminated before returning a result"))?
    }

    /// Send a status update via the process event bus.
    fn send_status(&self, status: &str) {
        let _ = self.event_tx.send(ProcessEvent::WorkerStatus {
            agent_id: self.agent_id.clone(),
            worker_id: self.id,
            channel_id: self.channel_id.clone(),
            status: status.to_string(),
        });
    }

    /// Check if the cancellation token has been triggered.
    fn ensure_not_cancelled(&self) -> anyhow::Result<()> {
        if self.cancellation_token.is_cancelled() {
            Err(anyhow::Error::new(Cancelled))
        } else {
            Ok(())
        }
    }
}
