//! ACP worker lifecycle: subprocess management, LocalSet thread, and result delivery.
//!
//! Each ACP worker spawns a dedicated `std::thread` running a single-threaded
//! tokio runtime with a `LocalSet` (required because the ACP `Client` trait is `!Send`).
//! Communication between the ACP thread and the main runtime uses:
//! - `broadcast::Sender<ProcessEvent>` for events (already `Send`)
//! - `mpsc::Receiver<String>` for follow-up input
//! - `oneshot::Sender` for the final result

use crate::acp::client::SpacebotAcpClient;
use crate::acp::types::{AcpWorkerResult, convert_acp_parts};
use crate::config::AcpProfileConfig;
use crate::{AgentId, ChannelId, ProcessEvent, WorkerId};

use agent_client_protocol::{
    Agent, ClientCapabilities, ClientSideConnection, ContentBlock, FileSystemCapability,
    Implementation, InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion,
    TextContent,
};
use futures::future::LocalBoxFuture;
use std::path::PathBuf;
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

/// An ACP-backed worker that drives a coding session via subprocess stdio.
pub struct AcpWorker {
    pub id: WorkerId,
    pub channel_id: Option<ChannelId>,
    pub agent_id: AgentId,
    pub task: String,
    pub directory: PathBuf,
    pub profile: AcpProfileConfig,
    pub event_tx: broadcast::Sender<ProcessEvent>,
    /// Input channel for interactive follow-ups.
    pub input_rx: Option<mpsc::Receiver<String>>,
    /// SQLite pool for incremental transcript persistence.
    pub sqlite_pool: Option<sqlx::SqlitePool>,
}

impl AcpWorker {
    /// Create a non-interactive ACP worker.
    pub fn new(
        channel_id: Option<ChannelId>,
        agent_id: AgentId,
        task: impl Into<String>,
        directory: PathBuf,
        profile: AcpProfileConfig,
        event_tx: broadcast::Sender<ProcessEvent>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel_id,
            agent_id,
            task: task.into(),
            directory,
            profile,
            event_tx,
            input_rx: None,
            sqlite_pool: None,
        }
    }

    /// Create an interactive ACP worker that accepts follow-up messages.
    pub fn new_interactive(
        channel_id: Option<ChannelId>,
        agent_id: AgentId,
        task: impl Into<String>,
        directory: PathBuf,
        profile: AcpProfileConfig,
        event_tx: broadcast::Sender<ProcessEvent>,
    ) -> (Self, mpsc::Sender<String>) {
        let (input_tx, input_rx) = mpsc::channel(32);
        let mut worker = Self::new(channel_id, agent_id, task, directory, profile, event_tx);
        worker.input_rx = Some(input_rx);
        (worker, input_tx)
    }

    /// Set the SQLite pool for transcript persistence.
    pub fn with_sqlite_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        self.sqlite_pool = Some(pool);
        self
    }

    /// Run the ACP worker.
    ///
    /// Spawns a dedicated thread with a single-threaded tokio runtime + LocalSet,
    /// then drives the ACP subprocess session to completion.
    pub async fn run(mut self) -> anyhow::Result<AcpWorkerResult> {
        let (result_tx, result_rx) = oneshot::channel();
        let input_rx = self.input_rx.take();
        let sqlite_pool = self.sqlite_pool.clone();

        let worker_id = self.id;
        let agent_id = self.agent_id.clone();
        let channel_id = self.channel_id.clone();
        let task = self.task.clone();
        let directory = self.directory.clone();
        let profile = self.profile.clone();
        let event_tx = self.event_tx.clone();
        let timeout_secs = profile.timeout;

        let thread_name = format!("acp-worker-{worker_id}");
        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build single-threaded tokio runtime for ACP worker");

                let local = tokio::task::LocalSet::new();
                let result = local.block_on(&rt, async move {
                    run_acp_session(
                        worker_id,
                        agent_id,
                        channel_id,
                        &task,
                        &directory,
                        &profile,
                        event_tx,
                        input_rx,
                        sqlite_pool,
                        timeout_secs,
                    )
                    .await
                });

                let _ = result_tx.send(result);
            })
            .map_err(|error| anyhow::anyhow!("failed to spawn ACP worker thread: {error}"))?;

        result_rx
            .await
            .map_err(|_| anyhow::anyhow!("ACP worker thread exited without sending result"))?
    }
}

/// Core ACP session logic running on a single-threaded LocalSet.
///
/// Uses `Rc<SpacebotAcpClient>` so the client state can be read back after
/// the prompt completes (the `Client` trait impl is `!Send`, which is fine
/// on a LocalSet).
#[allow(clippy::too_many_arguments)]
async fn run_acp_session(
    worker_id: WorkerId,
    agent_id: AgentId,
    channel_id: Option<ChannelId>,
    task: &str,
    directory: &PathBuf,
    profile: &AcpProfileConfig,
    event_tx: broadcast::Sender<ProcessEvent>,
    mut input_rx: Option<mpsc::Receiver<String>>,
    sqlite_pool: Option<sqlx::SqlitePool>,
    timeout_secs: u64,
) -> anyhow::Result<AcpWorkerResult> {
    use std::rc::Rc;

    // 1. Spawn subprocess.
    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "starting subprocess");

    let mut child = tokio::process::Command::new(&profile.command)
        .args(&profile.args)
        .envs(&profile.env)
        .current_dir(directory)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to spawn ACP subprocess '{}': {error}",
                profile.command
            )
        })?;

    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture subprocess stdin"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture subprocess stdout"))?;
    if let Some(stderr) = child.stderr.take() {
        tokio::task::spawn_local(drain_stderr(worker_id, stderr));
    }

    // child will be killed explicitly in cleanup paths and at the end.

    // 2. Adapt stdio to futures AsyncRead/AsyncWrite.
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
    let stdin_compat = child_stdin.compat_write();
    let stdout_compat = child_stdout.compat();

    // 3. Create client with shared Rc so we can read state after prompt completes.
    let client = Rc::new(SpacebotAcpClient::new(
        worker_id,
        agent_id.clone(),
        channel_id.clone(),
        event_tx.clone(),
        directory.clone(),
    ));
    let client_ref = Rc::clone(&client);

    let (connection, io_task) = ClientSideConnection::new(
        client,
        stdin_compat,
        stdout_compat,
        |fut: LocalBoxFuture<'static, ()>| {
            tokio::task::spawn_local(fut);
        },
    );

    let io_handle = tokio::task::spawn_local(async move {
        if let Err(error) = io_task.await {
            tracing::debug!(worker_id = %worker_id, %error, "ACP io_task ended");
        }
    });

    // 4. Initialize.
    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "initializing");

    let init_result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        connection.initialize(
            InitializeRequest::new(ProtocolVersion::LATEST)
                .client_capabilities(
                    ClientCapabilities::new()
                        .fs(
                            FileSystemCapability::new()
                                .read_text_file(true)
                                .write_text_file(true),
                        )
                        .terminal(false),
                )
                .client_info(Implementation::new("spacebot", env!("CARGO_PKG_VERSION"))),
        ),
    )
    .await;

    match init_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return Err(anyhow::anyhow!("ACP initialize failed: {error}")),
        Err(_) => return Err(anyhow::anyhow!("ACP initialize timed out after 30s")),
    };

    // 5. Create session.
    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "creating session");

    let session_response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        connection.new_session(NewSessionRequest::new(directory)),
    )
    .await;

    let session_id = match session_response {
        Ok(Ok(response)) => response.session_id,
        Ok(Err(error)) => return Err(anyhow::anyhow!("ACP new_session failed: {error}")),
        Err(_) => return Err(anyhow::anyhow!("ACP new_session timed out after 30s")),
    };

    tracing::info!(
        worker_id = %worker_id,
        session_id = %session_id,
        command = %profile.command,
        "ACP session created"
    );

    // 6. Send initial prompt.
    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "sending task");

    let prompt_result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        connection.prompt(PromptRequest::new(
            session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(task))],
        )),
    )
    .await;

    match &prompt_result {
        Ok(Ok(_)) => {
            tracing::info!(worker_id = %worker_id, "ACP initial prompt completed");
        }
        Ok(Err(error)) => {
            return Err(anyhow::anyhow!("ACP prompt failed: {error}"));
        }
        Err(_) => {
            let _ = connection
                .cancel(agent_client_protocol::CancelNotification::new(
                    session_id.clone(),
                ))
                .await;
            return Err(anyhow::anyhow!(
                "ACP prompt timed out after {timeout_secs}s"
            ));
        }
    }

    // 7. Interactive follow-up loop.
    if let Some(ref mut rx) = input_rx {
        let (result_text, _, _) = client_ref.take_result();
        if !result_text.is_empty() {
            let _ = event_tx.send(ProcessEvent::WorkerInitialResult {
                agent_id: agent_id.clone(),
                worker_id,
                channel_id: channel_id.clone(),
                result: result_text,
            });
        }

        persist_transcript_snapshot(&client_ref, worker_id, &sqlite_pool).await;
        send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "waiting for follow-up");
        let _ = event_tx.send(ProcessEvent::WorkerIdle {
            agent_id: agent_id.clone(),
            worker_id,
            channel_id: channel_id.clone(),
        });

        while let Some(follow_up) = rx.recv().await {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(anyhow::anyhow!(
                    "ACP subprocess exited with {status} before follow-up could be sent"
                ));
            }

            send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "processing follow-up");

            let follow_up_result = tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                connection.prompt(PromptRequest::new(
                    session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new(&follow_up))],
                )),
            )
            .await;

            match follow_up_result {
                Ok(Ok(_)) => {
                    let (follow_up_text, _, _) = client_ref.take_result();
                    if !follow_up_text.is_empty() {
                        let _ = event_tx.send(ProcessEvent::WorkerInitialResult {
                            agent_id: agent_id.clone(),
                            worker_id,
                            channel_id: channel_id.clone(),
                            result: follow_up_text,
                        });
                    }
                    persist_transcript_snapshot(&client_ref, worker_id, &sqlite_pool).await;
                    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "waiting for follow-up");
                    let _ = event_tx.send(ProcessEvent::WorkerIdle {
                        agent_id: agent_id.clone(),
                        worker_id,
                        channel_id: channel_id.clone(),
                    });
                }
                Ok(Err(error)) => {
                    tracing::error!(worker_id = %worker_id, %error, "ACP follow-up failed");
                    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "failed");
                    break;
                }
                Err(_) => {
                    let _ = connection
                        .cancel(agent_client_protocol::CancelNotification::new(
                            session_id.clone(),
                        ))
                        .await;
                    tracing::error!(worker_id = %worker_id, "ACP follow-up timed out");
                    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "timed out");
                    break;
                }
            }
        }
    }

    send_status(&event_tx, &agent_id, worker_id, channel_id.as_ref(), "completed");

    // 8. Extract final result from shared client state.
    let (result_text, parts, tool_calls) = client_ref.take_result();
    let transcript = convert_acp_parts(&parts);

    io_handle.abort();
    let _ = child.start_kill();

    tracing::info!(
        worker_id = %worker_id,
        transcript_steps = transcript.len(),
        tool_calls,
        "ACP worker completed"
    );

    Ok(AcpWorkerResult {
        result_text,
        transcript,
        tool_calls,
    })
}

/// Persist a snapshot of the current transcript to the DB.
async fn persist_transcript_snapshot(
    client: &SpacebotAcpClient,
    worker_id: WorkerId,
    sqlite_pool: &Option<sqlx::SqlitePool>,
) {
    let Some(pool) = sqlite_pool else { return };
    let (_, parts, tool_calls) = client.take_result();
    if parts.is_empty() {
        return;
    }
    let steps = convert_acp_parts(&parts);
    if steps.is_empty() {
        return;
    }
    let blob = crate::conversation::worker_transcript::serialize_steps(&steps);
    let wid = worker_id.to_string();
    if let Err(error) =
        sqlx::query("UPDATE worker_runs SET transcript = ?, tool_calls = ? WHERE id = ?")
            .bind(&blob)
            .bind(tool_calls)
            .bind(&wid)
            .execute(pool)
            .await
    {
        tracing::warn!(%error, worker_id = wid, "failed to persist ACP transcript snapshot");
    }
}

/// Send a WorkerStatus event.
fn send_status(
    event_tx: &broadcast::Sender<ProcessEvent>,
    agent_id: &AgentId,
    worker_id: WorkerId,
    channel_id: Option<&ChannelId>,
    status: &str,
) {
    let _ = event_tx.send(ProcessEvent::WorkerStatus {
        agent_id: agent_id.clone(),
        worker_id,
        channel_id: channel_id.cloned(),
        status: status.to_string(),
    });
}

/// Drain stderr from the subprocess to prevent buffer overflow.
async fn drain_stderr(worker_id: WorkerId, stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;
    let mut stderr = stderr;
    let mut buf = vec![0u8; 4096];
    let mut total = 0usize;
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if total <= 4096 {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    tracing::debug!(
                        worker_id = %worker_id,
                        stderr = %text.trim(),
                        "ACP subprocess stderr"
                    );
                }
            }
            Err(_) => break,
        }
    }
    if total > 4096 {
        tracing::debug!(
            worker_id = %worker_id,
            total_stderr_bytes = total,
            "ACP subprocess stderr drained"
        );
    }
}
