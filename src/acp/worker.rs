//! ACP worker runtime.

use crate::acp::bridge::AcpCmd;
use crate::acp::{AcpProfile, AcpUpdate, spawn_acp_bridge};
use crate::error::AgentError;
use crate::{AgentId, ChannelId, ProcessEvent, WorkerId};

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// ACP-backed interactive worker.
pub struct AcpWorker {
    pub id: WorkerId,
    channel_id: Option<ChannelId>,
    agent_id: AgentId,
    task: String,
    directory: PathBuf,
    profile: AcpProfile,
    event_tx: tokio::sync::broadcast::Sender<ProcessEvent>,
    sqlite_pool: sqlx::SqlitePool,
    input_rx: tokio::sync::mpsc::Receiver<AcpCmd>,
    handshake_timeout_secs: u64,
    stderr_buffer_bytes: usize,
}

impl AcpWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new_interactive(
        channel_id: Option<ChannelId>,
        agent_id: AgentId,
        task: impl Into<String>,
        directory: PathBuf,
        profile: AcpProfile,
        event_tx: tokio::sync::broadcast::Sender<ProcessEvent>,
        sqlite_pool: sqlx::SqlitePool,
        handshake_timeout_secs: u64,
        stderr_buffer_bytes: usize,
    ) -> (Self, tokio::sync::mpsc::Sender<AcpCmd>) {
        let (input_tx, input_rx) = tokio::sync::mpsc::channel(32);
        let worker = Self {
            id: uuid::Uuid::new_v4(),
            channel_id,
            agent_id,
            task: task.into(),
            directory,
            profile,
            event_tx,
            sqlite_pool,
            input_rx,
            handshake_timeout_secs,
            stderr_buffer_bytes,
        };
        (worker, input_tx)
    }

    async fn persist_snapshot(&self, updates: &[AcpUpdate]) {
        let (steps, _result) = crate::conversation::worker_transcript::convert_acp_updates(updates);
        let transcript = crate::conversation::worker_transcript::serialize_steps(&steps);
        let tool_calls = updates
            .iter()
            .filter(|update| matches!(update, AcpUpdate::ToolCall { .. }))
            .count() as i64;
        let worker_id = self.id.to_string();
        let pool = self.sqlite_pool.clone();
        tokio::spawn(async move {
            if let Err(error) =
                sqlx::query("UPDATE worker_runs SET transcript = ?, tool_calls = ? WHERE id = ?")
                    .bind(&transcript)
                    .bind(tool_calls)
                    .bind(&worker_id)
                    .execute(&pool)
                    .await
            {
                tracing::warn!(%error, worker_id, "failed to persist ACP transcript snapshot");
            }
        });
    }

    async fn join_bridge(handle: std::thread::JoinHandle<()>) {
        let _ = tokio::task::spawn_blocking(move || {
            let _ = handle.join();
        })
        .await;
    }

    pub async fn run(mut self) -> crate::Result<String> {
        let (bridge_tx, mut bridge_rx, bridge_handle) = spawn_acp_bridge(
            self.profile.clone(),
            self.directory.clone(),
            Duration::from_secs(self.handshake_timeout_secs),
            self.stderr_buffer_bytes,
        );

        let mut updates = Vec::new();
        let mut prompt_result = String::new();
        let mut awaiting_input = false;
        let mut pending_initial_prompt = Some(self.task.clone());
        let mut last_persist = Instant::now();
        let mut session_created = false;

        loop {
            tokio::select! {
                maybe_event = bridge_rx.recv() => {
                    let Some(event) = maybe_event else {
                        Self::join_bridge(bridge_handle).await;
                        return Err(AgentError::Other(anyhow::anyhow!("ACP bridge disconnected")).into());
                    };

                    match event {
                        crate::acp::AcpEvt::Initialized => {}
                        crate::acp::AcpEvt::SessionCreated { session_id } => {
                            session_created = true;
                            let _ = self.event_tx.send(ProcessEvent::AcpSessionCreated {
                                agent_id: self.agent_id.clone(),
                                worker_id: self.id,
                                channel_id: self.channel_id.clone(),
                                session_id,
                                profile_id: self.profile.id.clone(),
                            });

                            if let Some(task) = pending_initial_prompt.take() {
                                bridge_tx.send(AcpCmd::Prompt(task)).await.map_err(|_| {
                                    AgentError::Other(anyhow::anyhow!("ACP bridge stopped before initial prompt"))
                                })?;
                            }
                        }
                        crate::acp::AcpEvt::SessionUpdate { update } => {
                            if let Some(text) = update.agent_text() {
                                prompt_result.push_str(text);
                            }
                            updates.push(update.clone());
                            let _ = self.event_tx.send(ProcessEvent::AcpUpdateReceived {
                                agent_id: self.agent_id.clone(),
                                worker_id: self.id,
                                update,
                            });
                            if updates.len() % 8 == 0 || last_persist.elapsed() >= Duration::from_secs(2) {
                                self.persist_snapshot(&updates).await;
                                last_persist = Instant::now();
                            }
                        }
                        crate::acp::AcpEvt::PermissionRequested { request_id, description, .. } => {
                            let _ = self.event_tx.send(ProcessEvent::WorkerPermission {
                                agent_id: self.agent_id.clone(),
                                worker_id: self.id,
                                channel_id: self.channel_id.clone(),
                                permission_id: request_id,
                                description,
                                patterns: Vec::new(),
                            });
                        }
                        crate::acp::AcpEvt::PromptFinished { stop_reason } => {
                            let step_finish = AcpUpdate::step_finish(stop_reason);
                            updates.push(step_finish.clone());
                            let _ = self.event_tx.send(ProcessEvent::AcpUpdateReceived {
                                agent_id: self.agent_id.clone(),
                                worker_id: self.id,
                                update: step_finish,
                            });
                            self.persist_snapshot(&updates).await;
                            last_persist = Instant::now();

                            match stop_reason {
                                agent_client_protocol::StopReason::EndTurn
                                | agent_client_protocol::StopReason::MaxTokens
                                | agent_client_protocol::StopReason::MaxTurnRequests => {
                                    let result_text = if prompt_result.trim().is_empty() {
                                        format!(
                                            "ACP worker ({}) completed without text output.",
                                            self.profile.id
                                        )
                                    } else {
                                        prompt_result.trim().to_string()
                                    };
                                    let _ = self.event_tx.send(ProcessEvent::WorkerInitialResult {
                                        agent_id: self.agent_id.clone(),
                                        worker_id: self.id,
                                        channel_id: self.channel_id.clone(),
                                        result: result_text.clone(),
                                    });
                                    let _ = self.event_tx.send(ProcessEvent::WorkerIdle {
                                        agent_id: self.agent_id.clone(),
                                        worker_id: self.id,
                                        channel_id: self.channel_id.clone(),
                                    });
                                    awaiting_input = true;
                                    prompt_result.clear();
                                }
                                agent_client_protocol::StopReason::Refusal => {
                                    bridge_tx.send(AcpCmd::Shutdown).await.ok();
                                    Self::join_bridge(bridge_handle).await;
                                    return Err(AgentError::Other(anyhow::anyhow!(
                                        "ACP agent refused the task"
                                    )).into());
                                }
                                agent_client_protocol::StopReason::Cancelled => {
                                    bridge_tx.send(AcpCmd::Shutdown).await.ok();
                                    Self::join_bridge(bridge_handle).await;
                                    return Err(AgentError::Cancelled {
                                        reason: "ACP session cancelled".into(),
                                    }
                                    .into());
                                }
                                _ => {
                                    bridge_tx.send(AcpCmd::Shutdown).await.ok();
                                    Self::join_bridge(bridge_handle).await;
                                    return Err(AgentError::Other(anyhow::anyhow!(
                                        "ACP agent stopped for an unknown reason"
                                    ))
                                    .into());
                                }
                            }
                        }
                        crate::acp::AcpEvt::Disconnected { reason } => {
                            Self::join_bridge(bridge_handle).await;
                            if session_created && awaiting_input {
                                return Ok(format!("ACP worker disconnected while idle: {reason}"));
                            }
                            return Err(AgentError::Other(anyhow::anyhow!(reason)).into());
                        }
                    }
                }
                maybe_cmd = self.input_rx.recv() => {
                    let Some(command) = maybe_cmd else {
                        if session_created {
                            let _ = bridge_tx.send(AcpCmd::Shutdown).await;
                        }
                        Self::join_bridge(bridge_handle).await;
                        return Ok("ACP worker input channel closed".into());
                    };

                    match command {
                        AcpCmd::Prompt(message) => {
                            if !awaiting_input {
                                tracing::debug!(worker_id = %self.id, "ignoring ACP follow-up while worker is active");
                                continue;
                            }
                            awaiting_input = false;
                            prompt_result.clear();
                            let _ = self.event_tx.send(ProcessEvent::WorkerStatus {
                                agent_id: self.agent_id.clone(),
                                worker_id: self.id,
                                channel_id: self.channel_id.clone(),
                                status: "processing follow-up".into(),
                            });
                            bridge_tx.send(AcpCmd::Prompt(message)).await.map_err(|_| {
                                AgentError::Other(anyhow::anyhow!("ACP bridge stopped before follow-up prompt"))
                            })?;
                        }
                        AcpCmd::Cancel => {
                            bridge_tx.send(AcpCmd::Cancel).await.ok();
                        }
                        AcpCmd::PermissionReply { request_id, option_id } => {
                            bridge_tx
                                .send(AcpCmd::PermissionReply { request_id, option_id })
                                .await
                                .ok();
                        }
                        AcpCmd::Shutdown => {
                            bridge_tx.send(AcpCmd::Shutdown).await.ok();
                            Self::join_bridge(bridge_handle).await;
                            return Ok("ACP worker shut down".into());
                        }
                    }
                }
            }
        }
    }
}
