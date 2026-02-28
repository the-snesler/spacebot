//! ACP subprocess lifecycle management.
//!
//! Handles spawning the ACP agent binary, piping stdin/stdout for the JSON-RPC
//! connection, capturing stderr for diagnostics, and graceful/forceful shutdown.

use crate::config::AcpAgentConfig;
use anyhow::{Context as _, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

/// Maximum bytes to capture from the agent's stderr stream.
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// A running ACP agent subprocess with piped I/O handles.
pub struct AcpProcess {
    child: Child,
    /// Piped stdin — given to `ClientSideConnection` for outgoing JSON-RPC.
    pub stdin: Option<ChildStdin>,
    /// Piped stdout — given to `ClientSideConnection` for incoming JSON-RPC.
    pub stdout: Option<ChildStdout>,
    /// Background task capturing stderr (capped at [`MAX_STDERR_BYTES`]).
    stderr_task: JoinHandle<String>,
}

impl AcpProcess {
    /// Spawn an ACP agent subprocess from the given profile config.
    ///
    /// The command is resolved (including `env:VAR_NAME` references) and launched
    /// with stdin/stdout piped for JSON-RPC and stderr captured in a background task.
    pub fn spawn(config: &AcpAgentConfig, working_dir: &Path) -> anyhow::Result<Self> {
        let command = resolve_command(&config.command)?;

        let mut cmd = tokio::process::Command::new(&command);
        cmd.args(&config.args)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn ACP agent: {command}"))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Capture stderr in a background task for diagnostics on failure.
        let stderr_task = tokio::spawn(async move {
            let Some(stderr) = stderr else {
                return String::new();
            };
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::with_capacity(4096);
            loop {
                let mut chunk = [0u8; 4096];
                match reader.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = MAX_STDERR_BYTES.saturating_sub(buf.len());
                        if remaining == 0 {
                            continue; // drain but don't store
                        }
                        buf.extend_from_slice(&chunk[..n.min(remaining)]);
                    }
                    Err(_) => break,
                }
            }
            String::from_utf8_lossy(&buf).into_owned()
        });

        Ok(Self {
            child,
            stdin,
            stdout,
            stderr_task,
        })
    }

    /// Send SIGTERM to the agent process, wait briefly, then SIGKILL if needed.
    pub async fn kill(&mut self) {
        // Try graceful termination first
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        // Wait up to 3 seconds for graceful exit
        match tokio::time::timeout(std::time::Duration::from_secs(3), self.child.wait()).await {
            Ok(_) => (),
            Err(_) => {
                // Force kill
                let _ = self.child.kill().await;
            }
        }
    }

    /// Collect captured stderr output. Useful for error diagnostics.
    ///
    /// This consumes the stderr task handle. Returns an empty string if
    /// stderr capture has already been consumed or the task panicked.
    pub async fn stderr_output(&mut self) -> String {
        (&mut self.stderr_task).await.unwrap_or_default()
    }

    /// Check if the subprocess is still running.
    pub fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}

/// Resolve a command string, handling `env:VAR_NAME` references.
fn resolve_command(raw: &str) -> anyhow::Result<String> {
    if let Some(var_name) = raw.strip_prefix("env:") {
        std::env::var(var_name).with_context(|| {
            format!("ACP command references env var '{var_name}' which is not set")
        })
    } else if raw.is_empty() {
        bail!("ACP command is empty")
    } else {
        Ok(raw.to_string())
    }
}
