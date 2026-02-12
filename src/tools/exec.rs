//! Exec tool for running subprocesses (task workers only).

use crate::error::Result;
use std::process::Stdio;
use tokio::process::Command;

/// Execute a subprocess with arguments.
pub async fn exec(
    program: &str,
    args: &[&str],
    working_dir: Option<&std::path::Path>,
    env: Option<&[(&str, &str)]>,
) -> Result<ExecResult> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    
    if let Some(environment) = env {
        for (key, value) in environment {
            cmd.env(key, value);
        }
    }
    
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());
    
    let output = cmd.output()
        .await
        .with_context(|| format!("failed to execute: {}", program))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok(ExecResult {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// Result of a subprocess execution.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

use anyhow::Context as _;
