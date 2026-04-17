//! ACP worker configuration.

use crate::error::{ConfigError, Result};

use serde::{Deserialize, Serialize};

use std::collections::{HashMap, HashSet};

/// ACP subprocess worker configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpConfig {
    /// Whether ACP workers are available.
    pub enabled: bool,
    /// Named ACP profiles available to the agent.
    pub profiles: Vec<AcpProfile>,
    /// Timeout waiting for initialize + session/new to complete.
    pub handshake_timeout_secs: u64,
    /// Maximum stderr bytes retained for failure diagnostics.
    pub stderr_buffer_bytes: usize,
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profiles: Vec::new(),
            handshake_timeout_secs: 20,
            stderr_buffer_bytes: 16 * 1024,
        }
    }
}

/// A named ACP subprocess profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpProfile {
    pub id: String,
    pub display_name: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Prompt-facing profile summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpProfileInfo {
    pub id: String,
    pub display_name: Option<String>,
}

impl From<&AcpProfile> for AcpProfileInfo {
    fn from(value: &AcpProfile) -> Self {
        Self {
            id: value.id.clone(),
            display_name: value.display_name.clone(),
        }
    }
}

/// Validate ACP configuration after TOML/env/secret resolution.
pub fn validate_acp_config(config: &AcpConfig) -> Result<()> {
    let mut seen_ids = HashSet::new();

    for profile in &config.profiles {
        let id = profile.id.trim();
        if id.is_empty() {
            return Err(ConfigError::Invalid("ACP profile id cannot be empty".into()).into());
        }
        if matches!(id, "builtin" | "opencode") {
            return Err(ConfigError::Invalid(format!(
                "ACP profile id `{id}` is reserved; choose another id"
            ))
            .into());
        }
        if !seen_ids.insert(id.to_string()) {
            return Err(ConfigError::Invalid(format!("duplicate ACP profile id `{id}`")).into());
        }
        if !id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        }) {
            tracing::warn!(
                profile_id = id,
                "ACP profile ids should use only [a-z0-9_-] for prompt/tool stability"
            );
        }
        if profile.command.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "ACP profile `{id}` must define a non-empty command"
            ))
            .into());
        }
    }

    Ok(())
}
