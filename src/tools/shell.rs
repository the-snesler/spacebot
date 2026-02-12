//! Shell tool for executing shell commands (task workers only).

use crate::error::Result;
use std::process::Stdio;
use tokio::process::Command;

/// Execute a shell command.
pub async fn shell(command: &str, working_dir: Option<&std::path::Path>) -> Result<ShellResult> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());
    
    let output = cmd.output()
        .await
        .map_err(|e| crate::error::AgentError::Other(e.into()))?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    Ok(ShellResult {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// Result of a shell command execution.
#[derive(Debug, Clone)]
pub struct ShellResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ShellResult {
    /// Format as a readable string for LLM consumption.
    pub fn format(&self) -> String {
        let mut output = String::new();
        
        output.push_str(&format!("Exit code: {}\n", self.exit_code));
        
        if !self.stdout.is_empty() {
            output.push_str("\nSTDOUT:\n");
            output.push_str(&self.stdout);
        }
        
        if !self.stderr.is_empty() {
            output.push_str("\nSTDERR:\n");
            output.push_str(&self.stderr);
        }
        
        output
    }
}
