//! Compactor: Programmatic context monitor.

use crate::error::Result;
use crate::{ChannelId, AgentDeps};
use crate::config::CompactionConfig;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Compaction thresholds.
pub const BACKGROUND_THRESHOLD: f32 = 0.80;
pub const AGGRESSIVE_THRESHOLD: f32 = 0.85;
pub const EMERGENCY_THRESHOLD: f32 = 0.95;

/// Programmatic monitor that watches channel context size.
pub struct Compactor {
    pub channel_id: ChannelId,
    pub config: CompactionConfig,
    pub deps: AgentDeps,
    /// Current context usage (0.0 - 1.0).
    pub usage: Arc<RwLock<f32>>,
    /// Is a compaction currently running.
    pub is_compacting: Arc<RwLock<bool>>,
}

impl Compactor {
    /// Create a new compactor for a channel.
    pub fn new(channel_id: ChannelId, config: CompactionConfig, deps: AgentDeps) -> Self {
        Self {
            channel_id,
            config,
            deps,
            usage: Arc::new(RwLock::new(0.0)),
            is_compacting: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Check context size and trigger compaction if needed.
    pub async fn check(&self, current_usage: f32) -> Result<Option<CompactionAction>> {
        let mut usage = self.usage.write().await;
        *usage = current_usage;
        drop(usage);
        
        let is_compacting = *self.is_compacting.read().await;
        
        if current_usage >= self.config.emergency_threshold {
            // Emergency: truncate immediately without LLM
            if !is_compacting {
                return Ok(Some(CompactionAction::EmergencyTruncate));
            }
        } else if current_usage >= self.config.aggressive_threshold {
            // Aggressive: urgent compaction
            if !is_compacting {
                return Ok(Some(CompactionAction::Aggressive));
            }
        } else if current_usage >= self.config.background_threshold {
            // Background: normal compaction
            if !is_compacting {
                return Ok(Some(CompactionAction::Background));
            }
        }
        
        Ok(None)
    }
    
    /// Trigger a compaction.
    pub async fn compact(&self, action: CompactionAction) -> Result<()> {
        let mut is_compacting = self.is_compacting.write().await;
        *is_compacting = true;
        drop(is_compacting);
        
        tracing::info!(
            channel_id = %self.channel_id,
            action = ?action,
            "starting compaction"
        );
        
        match action {
            CompactionAction::Background => {
                self.run_compaction_worker(false).await?;
            }
            CompactionAction::Aggressive => {
                self.run_compaction_worker(true).await?;
            }
            CompactionAction::EmergencyTruncate => {
                self.emergency_truncate().await?;
            }
        }
        
        let mut is_compacting = self.is_compacting.write().await;
        *is_compacting = false;
        
        Ok(())
    }
    
    /// Run a compaction worker (in background, non-blocking).
    async fn run_compaction_worker(&self, aggressive: bool) -> Result<()> {
        let _deps = self.deps.clone();
        let _channel_id = self.channel_id.clone();
        let _aggressive = aggressive;
        
        // Spawn the compaction worker without blocking
        tokio::spawn(async move {
            // In real implementation:
            // 1. Read old conversation turns from history
            // 2. Archive raw transcript to conversation_archives table
            // 3. Run LLM to summarize and extract memories
            // 4. Replace old turns with summary in channel history
            // 5. Send CompactionComplete event
            
            tracing::info!("compaction worker running");
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            tracing::info!("compaction complete");
        });
        
        Ok(())
    }
    
    /// Emergency truncation (no LLM, just drop oldest turns).
    async fn emergency_truncate(&self) -> Result<()> {
        // In real implementation:
        // 1. Remove oldest 50% of conversation turns
        // 2. Add a note that truncation occurred
        // 3. This is fast and blocking, but only happens at 95%
        
        tracing::warn!(channel_id = %self.channel_id, "emergency truncation performed");
        
        Ok(())
    }
}

/// Types of compaction actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionAction {
    /// Normal background compaction.
    Background,
    /// Aggressive compaction (more urgent).
    Aggressive,
    /// Emergency truncation (no LLM, just drop).
    EmergencyTruncate,
}
