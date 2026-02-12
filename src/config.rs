//! Configuration loading and validation.

use crate::error::{ConfigError, Result};
use anyhow::Context as _;
use std::path::Path;

/// Spacebot configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Data directory path.
    pub data_dir: std::path::PathBuf,

    /// LLM provider configuration.
    pub llm: LlmConfig,

    /// Compaction thresholds.
    pub compaction: CompactionConfig,

    /// Channel behavior settings.
    pub channel: ChannelConfig,
}

/// LLM provider configuration.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Default model name for channels.
    pub default_channel_model: String,

    /// Default model name for workers.
    pub default_worker_model: String,

    /// Anthropic API key (from env or secrets).
    pub anthropic_key: Option<String>,

    /// OpenAI API key (from env or secrets).
    pub openai_key: Option<String>,
}

/// Compaction threshold configuration.
#[derive(Debug, Clone, Copy)]
pub struct CompactionConfig {
    /// Threshold for background compaction (0.0 - 1.0).
    pub background_threshold: f32,

    /// Threshold for aggressive compaction.
    pub aggressive_threshold: f32,

    /// Threshold for emergency truncation.
    pub emergency_threshold: f32,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            background_threshold: 0.80,
            aggressive_threshold: 0.85,
            emergency_threshold: 0.95,
        }
    }
}

/// Channel behavior configuration.
#[derive(Debug, Clone, Copy)]
pub struct ChannelConfig {
    /// Maximum concurrent branches per channel.
    pub max_concurrent_branches: usize,

    /// Maximum turns for channel LLM calls.
    pub max_turns: usize,

    /// Context window size in tokens.
    pub context_window: usize,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            max_concurrent_branches: 5,
            max_turns: 5,
            context_window: 128_000,
        }
    }
}

impl Config {
    /// Load configuration from environment and config files.
    pub fn load() -> Result<Self> {
        let data_dir = dirs::data_dir()
            .map(|d| d.join("spacebot"))
            .unwrap_or_else(|| std::path::PathBuf::from("./data"));

        // Ensure data directory exists
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("failed to create data directory: {}", data_dir.display()))?;

        // Load LLM configuration
        let llm = LlmConfig {
            default_channel_model: std::env::var("SPACEBOT_CHANNEL_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4-20250514".into()),
            default_worker_model: std::env::var("SPACEBOT_WORKER_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4-20250514".into()),
            anthropic_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_key: std::env::var("OPENAI_API_KEY").ok(),
        };

        // Validate that at least one provider key is available
        if llm.anthropic_key.is_none() && llm.openai_key.is_none() {
            return Err(ConfigError::Invalid(
                "No LLM provider API key found. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.".into(),
            )
            .into());
        }

        let compaction = CompactionConfig::default();
        let channel = ChannelConfig::default();

        Ok(Self {
            data_dir,
            llm,
            compaction,
            channel,
        })
    }

    /// Load from a specific config file path.
    pub fn load_from_path(_path: &Path) -> Result<Self> {
        // For now, just use env-based loading
        // TODO: Add TOML file loading support
        Self::load()
    }

    /// Get the SQLite database path.
    pub fn sqlite_path(&self) -> std::path::PathBuf {
        self.data_dir.join("spacebot.db")
    }

    /// Get the LanceDB path.
    pub fn lancedb_path(&self) -> std::path::PathBuf {
        self.data_dir.join("lancedb")
    }

    /// Get the redb path.
    pub fn redb_path(&self) -> std::path::PathBuf {
        self.data_dir.join("config.redb")
    }
}
