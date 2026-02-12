//! Conversation history persistence (SQLite).

use crate::error::Result;
use crate::{ChannelId, InboundMessage, OutboundResponse};
use anyhow::Context as _;
use sqlx::SqlitePool;

/// History store for conversations.
pub struct HistoryStore {
    pool: SqlitePool,
}

/// A conversation turn (message + response pair).
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub id: String,
    pub channel_id: ChannelId,
    pub sequence: i64,
    pub inbound_message: String,
    pub outbound_response: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Summary of a compaction (replaces raw turns).
#[derive(Debug, Clone)]
pub struct CompactionSummary {
    pub id: String,
    pub channel_id: ChannelId,
    pub start_sequence: i64,
    pub end_sequence: i64,
    pub summary_text: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl HistoryStore {
    /// Create a new history store.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    
    /// Initialize the history tables.
    pub async fn initialize(&self) -> Result<()> {
        // Conversation turns table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS conversation_turns (
                id TEXT PRIMARY KEY,
                channel_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                inbound_message TEXT NOT NULL,
                outbound_response TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(channel_id, sequence)
            )
            "#
        )
        .execute(&self.pool)
        .await
        .context("failed to create conversation_turns table")?;
        
        // Compaction summaries table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS compaction_summaries (
                id TEXT PRIMARY KEY,
                channel_id TEXT NOT NULL,
                start_sequence INTEGER NOT NULL,
                end_sequence INTEGER NOT NULL,
                summary_text TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#
        )
        .execute(&self.pool)
        .await
        .context("failed to create compaction_summaries table")?;
        
        // Create indices
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_turns_channel ON conversation_turns(channel_id)")
            .execute(&self.pool)
            .await?;
        
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_turns_sequence ON conversation_turns(channel_id, sequence)")
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    /// Save a conversation turn.
    pub async fn save_turn(
        &self,
        channel_id: &ChannelId,
        sequence: i64,
        inbound: &str,
        outbound: Option<&str>,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        
        sqlx::query(
            r#"
            INSERT INTO conversation_turns (id, channel_id, sequence, inbound_message, outbound_response)
            VALUES (?, ?, ?, ?, ?)
            "#
        )
        .bind(&id)
        .bind(channel_id.as_ref())
        .bind(sequence)
        .bind(inbound)
        .bind(outbound)
        .execute(&self.pool)
        .await
        .context("failed to save conversation turn")?;
        
        Ok(())
    }
    
    /// Get the next sequence number for a channel.
    pub async fn next_sequence(&self, channel_id: &ChannelId) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(MAX(sequence), 0) + 1 as next_seq FROM conversation_turns WHERE channel_id = ?"
        )
        .bind(channel_id.as_ref())
        .fetch_one(&self.pool)
        .await
        .context("failed to get next sequence")?;
        
        let next_seq: i64 = row.try_get("next_seq")?;
        Ok(next_seq)
    }
    
    /// Load recent conversation history for a channel.
    pub async fn load_recent(&self, channel_id: &ChannelId, limit: i64) -> Result<Vec<ConversationTurn>> {
        let rows = sqlx::query(
            r#"
            SELECT id, channel_id, sequence, inbound_message, outbound_response, created_at
            FROM conversation_turns
            WHERE channel_id = ?
            ORDER BY sequence DESC
            LIMIT ?
            "#
        )
        .bind(channel_id.as_ref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("failed to load conversation history")?;
        
        let mut turns: Vec<ConversationTurn> = rows
            .into_iter()
            .map(|row| ConversationTurn {
                id: row.try_get("id").unwrap_or_default(),
                channel_id: ChannelId::from(row.try_get::<String, _>("channel_id").unwrap_or_default()),
                sequence: row.try_get("sequence").unwrap_or(0),
                inbound_message: row.try_get("inbound_message").unwrap_or_default(),
                outbound_response: row.try_get("outbound_response").ok(),
                created_at: row.try_get("created_at").unwrap_or_else(|_| chrono::Utc::now()),
            })
            .collect();
        
        // Reverse to get chronological order
        turns.reverse();
        
        Ok(turns)
    }
    
    /// Save a compaction summary.
    pub async fn save_compaction_summary(
        &self,
        channel_id: &ChannelId,
        start_sequence: i64,
        end_sequence: i64,
        summary: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        
        sqlx::query(
            r#"
            INSERT INTO compaction_summaries (id, channel_id, start_sequence, end_sequence, summary_text)
            VALUES (?, ?, ?, ?, ?)
            "#
        )
        .bind(&id)
        .bind(channel_id.as_ref())
        .bind(start_sequence)
        .bind(end_sequence)
        .bind(summary)
        .execute(&self.pool)
        .await
        .context("failed to save compaction summary")?;
        
        Ok(())
    }
    
    /// Load compaction summaries for a channel (oldest first).
    pub async fn load_summaries(&self, channel_id: &ChannelId) -> Result<Vec<CompactionSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT id, channel_id, start_sequence, end_sequence, summary_text, created_at
            FROM compaction_summaries
            WHERE channel_id = ?
            ORDER BY start_sequence ASC
            "#
        )
        .bind(channel_id.as_ref())
        .fetch_all(&self.pool)
        .await
        .context("failed to load compaction summaries")?;
        
        let summaries = rows
            .into_iter()
            .map(|row| CompactionSummary {
                id: row.try_get("id").unwrap_or_default(),
                channel_id: ChannelId::from(row.try_get::<String, _>("channel_id").unwrap_or_default()),
                start_sequence: row.try_get("start_sequence").unwrap_or(0),
                end_sequence: row.try_get("end_sequence").unwrap_or(0),
                summary_text: row.try_get("summary_text").unwrap_or_default(),
                created_at: row.try_get("created_at").unwrap_or_else(|_| chrono::Utc::now()),
            })
            .collect();
        
        Ok(summaries)
    }
}

use sqlx::Row as _;
