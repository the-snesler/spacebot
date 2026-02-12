//! Heartbeat CRUD storage (SQLite).

use crate::error::Result;
use crate::heartbeat::scheduler::HeartbeatConfig;
use anyhow::Context as _;
use sqlx::SqlitePool;

/// Heartbeat store for persistence.
pub struct HeartbeatStore {
    pool: SqlitePool,
}

impl HeartbeatStore {
    /// Create a new heartbeat store.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    
    /// Initialize the heartbeat tables.
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS heartbeats (
                id TEXT PRIMARY KEY,
                prompt TEXT NOT NULL,
                interval_secs INTEGER NOT NULL DEFAULT 3600,
                delivery_target TEXT NOT NULL,
                active_start_hour INTEGER,
                active_end_hour INTEGER,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#
        )
        .execute(&self.pool)
        .await
        .context("failed to create heartbeats table")?;
        
        // Create execution log table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS heartbeat_executions (
                id TEXT PRIMARY KEY,
                heartbeat_id TEXT NOT NULL,
                executed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                success INTEGER NOT NULL,
                result_summary TEXT,
                FOREIGN KEY (heartbeat_id) REFERENCES heartbeats(id) ON DELETE CASCADE
            )
            "#
        )
        .execute(&self.pool)
        .await
        .context("failed to create heartbeat_executions table")?;
        
        Ok(())
    }
    
    /// Save a heartbeat configuration.
    pub async fn save(&self, config: &HeartbeatConfig) -> Result<()> {
        let active_start = config.active_hours.map(|h| h.0 as i64);
        let active_end = config.active_hours.map(|h| h.1 as i64);
        
        sqlx::query(
            r#"
            INSERT INTO heartbeats (id, prompt, interval_secs, delivery_target, active_start_hour, active_end_hour, enabled)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                prompt = excluded.prompt,
                interval_secs = excluded.interval_secs,
                delivery_target = excluded.delivery_target,
                active_start_hour = excluded.active_start_hour,
                active_end_hour = excluded.active_end_hour,
                enabled = excluded.enabled
            "#
        )
        .bind(&config.id)
        .bind(&config.prompt)
        .bind(config.interval_secs as i64)
        .bind(&config.delivery_target)
        .bind(active_start)
        .bind(active_end)
        .bind(config.enabled as i64)
        .execute(&self.pool)
        .await
        .context("failed to save heartbeat")?;
        
        Ok(())
    }
    
    /// Load all heartbeat configurations.
    pub async fn load_all(&self) -> Result<Vec<HeartbeatConfig>> {
        let rows = sqlx::query(
            r#"
            SELECT id, prompt, interval_secs, delivery_target, active_start_hour, active_end_hour, enabled
            FROM heartbeats
            WHERE enabled = 1
            ORDER BY created_at ASC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to load heartbeats")?;
        
        let configs = rows
            .into_iter()
            .map(|row| HeartbeatConfig {
                id: row.try_get("id").unwrap_or_default(),
                prompt: row.try_get("prompt").unwrap_or_default(),
                interval_secs: row.try_get::<i64, _>("interval_secs").unwrap_or(3600) as u64,
                delivery_target: row.try_get("delivery_target").unwrap_or_default(),
                active_hours: {
                    let start: Option<i64> = row.try_get("active_start_hour").ok();
                    let end: Option<i64> = row.try_get("active_end_hour").ok();
                    match (start, end) {
                        (Some(s), Some(e)) => Some((s as u8, e as u8)),
                        _ => None,
                    }
                },
                enabled: row.try_get::<i64, _>("enabled").unwrap_or(1) != 0,
            })
            .collect();
        
        Ok(configs)
    }
    
    /// Delete a heartbeat.
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM heartbeats WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("failed to delete heartbeat")?;
        
        Ok(())
    }
}

use sqlx::Row as _;
