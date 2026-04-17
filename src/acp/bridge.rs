//! ACP bridge running the `!Send` client connection on a dedicated thread.

use crate::acp::AcpProfile;
use crate::acp::client::{BridgeClient, PendingPermissionReplies};
use crate::acp::types::AcpUpdate;

use agent_client_protocol::{self as acp, Agent as _};
use anyhow::Context as _;
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Commands sent from the worker runtime into the ACP bridge.
#[derive(Debug, Clone)]
pub enum AcpCmd {
    Prompt(String),
    Cancel,
    PermissionReply {
        request_id: String,
        option_id: Option<String>,
    },
    Shutdown,
}

/// Permission option surfaced by ACP.
#[derive(Debug, Clone)]
pub struct AcpPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

/// Events emitted from the ACP bridge back to the worker runtime.
#[derive(Debug, Clone)]
pub enum AcpEvt {
    Initialized,
    SessionCreated {
        session_id: String,
    },
    SessionUpdate {
        update: AcpUpdate,
    },
    PermissionRequested {
        request_id: String,
        description: String,
        options: Vec<AcpPermissionOption>,
    },
    PromptFinished {
        stop_reason: acp::StopReason,
    },
    Disconnected {
        reason: String,
    },
}

fn stderr_tail_push(buffer: &mut VecDeque<u8>, chunk: &[u8], max_bytes: usize) {
    if max_bytes == 0 {
        return;
    }
    for byte in chunk {
        if buffer.len() >= max_bytes {
            buffer.pop_front();
        }
        buffer.push_back(*byte);
    }
}

fn stderr_tail_string(buffer: &Rc<RefCell<VecDeque<u8>>>) -> String {
    let bytes: Vec<u8> = buffer.borrow().iter().copied().collect();
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    if text.is_empty() {
        String::new()
    } else {
        format!("\n\nstderr tail:\n{text}")
    }
}

async fn run_bridge(
    profile: AcpProfile,
    cwd: PathBuf,
    handshake_timeout: Duration,
    stderr_buffer_bytes: usize,
    mut cmd_rx: mpsc::Receiver<AcpCmd>,
    evt_tx: mpsc::Sender<AcpEvt>,
) -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new(&profile.command)
        .args(&profile.args)
        .envs(&profile.env)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn ACP profile `{}`", profile.id))?;

    let outgoing = child
        .stdin
        .take()
        .context("ACP child stdin unavailable")?
        .compat_write();
    let incoming = child
        .stdout
        .take()
        .context("ACP child stdout unavailable")?
        .compat();
    let stderr = child
        .stderr
        .take()
        .context("ACP child stderr unavailable")?;

    let stderr_tail = Rc::new(RefCell::new(VecDeque::new()));
    let stderr_tail_reader = stderr_tail.clone();
    tokio::task::spawn_local(async move {
        let mut stderr = stderr;
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    stderr_tail_push(
                        &mut stderr_tail_reader.borrow_mut(),
                        &buffer[..read],
                        stderr_buffer_bytes,
                    );
                }
                Err(error) => {
                    tracing::debug!(%error, "ACP stderr reader stopped");
                    break;
                }
            }
        }
    });

    let pending_replies: PendingPermissionReplies = Rc::new(RefCell::new(HashMap::new()));
    let bridge_client = BridgeClient::new(
        evt_tx.clone(),
        pending_replies.clone(),
        Duration::from_secs(1),
    );
    let (connection, handle_io) =
        acp::ClientSideConnection::new(bridge_client, outgoing, incoming, |future| {
            tokio::task::spawn_local(future);
        });
    tokio::task::spawn_local(handle_io);

    tokio::time::timeout(
        handshake_timeout,
        connection.initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1)
                .client_capabilities(acp::ClientCapabilities::new().terminal(false))
                .client_info(
                    acp::Implementation::new("spacebot", env!("CARGO_PKG_VERSION"))
                        .title("Spacebot ACP Bridge"),
                ),
        ),
    )
    .await
    .context("ACP initialize timed out")?
    .context("ACP initialize failed")?;

    evt_tx.send(AcpEvt::Initialized).await.ok();

    let session = tokio::time::timeout(
        handshake_timeout,
        connection.new_session(acp::NewSessionRequest::new(&cwd)),
    )
    .await
    .context("ACP session/new timed out")?
    .context("ACP session/new failed")?;
    evt_tx
        .send(AcpEvt::SessionCreated {
            session_id: session.session_id.to_string(),
        })
        .await
        .ok();

    while let Some(command) = cmd_rx.recv().await {
        match command {
            AcpCmd::Prompt(message) => {
                let response = connection
                    .prompt(acp::PromptRequest::new(
                        session.session_id.clone(),
                        vec![message.into()],
                    ))
                    .await
                    .map_err(anyhow::Error::from)?;
                evt_tx
                    .send(AcpEvt::PromptFinished {
                        stop_reason: response.stop_reason,
                    })
                    .await
                    .ok();
            }
            AcpCmd::Cancel => {
                connection
                    .cancel(acp::CancelNotification::new(session.session_id.clone()))
                    .await
                    .map_err(anyhow::Error::from)?;
            }
            AcpCmd::PermissionReply {
                request_id,
                option_id,
            } => {
                if let Some(sender) = pending_replies.borrow_mut().remove(&request_id) {
                    let _ = sender.send(option_id);
                }
            }
            AcpCmd::Shutdown => break,
        }
    }

    drop(connection);
    drop(child);
    let _ = evt_tx
        .send(AcpEvt::Disconnected {
            reason: format!("ACP bridge shut down{}", stderr_tail_string(&stderr_tail)),
        })
        .await;

    Ok(())
}

/// Spawn the ACP bridge on a dedicated OS thread.
pub fn spawn_acp_bridge(
    profile: AcpProfile,
    cwd: PathBuf,
    handshake_timeout: Duration,
    stderr_buffer_bytes: usize,
) -> (mpsc::Sender<AcpCmd>, mpsc::Receiver<AcpEvt>, JoinHandle<()>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (evt_tx, evt_rx) = mpsc::channel(128);

    let join_handle = std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = evt_tx.blocking_send(AcpEvt::Disconnected {
                    reason: format!("failed to start ACP runtime: {error}"),
                });
                return;
            }
        };

        let local_set = tokio::task::LocalSet::new();
        let result = runtime.block_on(local_set.run_until(run_bridge(
            profile,
            cwd,
            handshake_timeout,
            stderr_buffer_bytes,
            cmd_rx,
            evt_tx.clone(),
        )));

        if let Err(error) = result {
            let _ = evt_tx.blocking_send(AcpEvt::Disconnected {
                reason: error.to_string(),
            });
        }
    });

    (cmd_tx, evt_rx, join_handle)
}
