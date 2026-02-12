//! Identity file loading: SOUL.md, IDENTITY.md, USER.md, and system prompts.

use crate::error::Result;
use anyhow::Context as _;
use std::path::PathBuf;

/// Load a prompt file from the prompts directory.
pub async fn load_prompt(name: &str) -> Result<String> {
    let path = PathBuf::from("prompts").join(format!("{}.md", name));
    
    tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to load prompt file: {}", path.display()))
        .map_err(Into::into)
}

/// Load the channel system prompt.
pub async fn channel_prompt() -> Result<String> {
    load_prompt("CHANNEL").await
}

/// Load the branch system prompt.
pub async fn branch_prompt() -> Result<String> {
    load_prompt("BRANCH").await
}

/// Load the worker system prompt.
pub async fn worker_prompt() -> Result<String> {
    load_prompt("WORKER").await
}

/// Load the cortex system prompt.
pub async fn cortex_prompt() -> Result<String> {
    load_prompt("CORTEX").await
}

/// Load the compactor system prompt.
pub async fn compactor_prompt() -> Result<String> {
    load_prompt("COMPACTOR").await
}

/// Load all prompts at startup.
pub async fn load_all_prompts() -> anyhow::Result<Prompts> {
    Ok(Prompts {
        channel: channel_prompt().await?,
        branch: branch_prompt().await?,
        worker: worker_prompt().await?,
        cortex: cortex_prompt().await?,
        compactor: compactor_prompt().await?,
    })
}

/// Container for all loaded prompts.
#[derive(Clone, Debug)]
pub struct Prompts {
    pub channel: String,
    pub branch: String,
    pub worker: String,
    pub cortex: String,
    pub compactor: String,
}

impl Prompts {
    /// Load all prompts from disk.
    pub async fn load() -> anyhow::Result<Self> {
        load_all_prompts().await
    }
}
