//! Heartbeat management tool for creating, listing, and deleting scheduled tasks.

use crate::heartbeat::scheduler::{HeartbeatConfig, Scheduler};
use crate::heartbeat::store::HeartbeatStore;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Tool for managing heartbeats (scheduled recurring tasks).
#[derive(Debug, Clone)]
pub struct HeartbeatTool {
    store: Arc<HeartbeatStore>,
    scheduler: Arc<Scheduler>,
}

impl HeartbeatTool {
    pub fn new(store: Arc<HeartbeatStore>, scheduler: Arc<Scheduler>) -> Self {
        Self { store, scheduler }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Heartbeat operation failed: {0}")]
pub struct HeartbeatError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HeartbeatArgs {
    /// The operation to perform: "create", "list", or "delete".
    pub action: String,
    /// Required for "create": a short unique ID for the heartbeat (e.g. "check-email", "daily-summary").
    #[serde(default)]
    pub id: Option<String>,
    /// Required for "create": the prompt/instruction to execute on each run.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Required for "create": interval in seconds between runs.
    #[serde(default)]
    pub interval_secs: Option<u64>,
    /// Required for "create": where to deliver results, in "adapter:target" format (e.g. "discord:123456789").
    #[serde(default)]
    pub delivery_target: Option<String>,
    /// Optional for "create": hour (0-23) when the heartbeat becomes active.
    #[serde(default)]
    pub active_start_hour: Option<u8>,
    /// Optional for "create": hour (0-23) when the heartbeat becomes inactive.
    #[serde(default)]
    pub active_end_hour: Option<u8>,
    /// Required for "delete": the ID of the heartbeat to remove.
    #[serde(default)]
    pub delete_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatOutput {
    pub success: bool,
    pub message: String,
    /// Populated on "list" action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeats: Option<Vec<HeartbeatEntry>>,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatEntry {
    pub id: String,
    pub prompt: String,
    pub interval_secs: u64,
    pub delivery_target: String,
    pub active_hours: Option<String>,
}

impl Tool for HeartbeatTool {
    const NAME: &'static str = "heartbeat";

    type Error = HeartbeatError;
    type Args = HeartbeatArgs;
    type Output = HeartbeatOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Manage scheduled recurring tasks (heartbeats). Use this to create, list, or delete heartbeats. A heartbeat runs a prompt on a timer and delivers the result to a messaging channel.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "list", "delete"],
                        "description": "The operation: create a new heartbeat, list all heartbeats, or delete one."
                    },
                    "id": {
                        "type": "string",
                        "description": "For 'create': a short unique ID (e.g. 'check-email', 'daily-summary')."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "For 'create': the instruction to execute on each run."
                    },
                    "interval_secs": {
                        "type": "integer",
                        "description": "For 'create': seconds between runs (e.g. 3600 = hourly, 86400 = daily)."
                    },
                    "delivery_target": {
                        "type": "string",
                        "description": "For 'create': where to send results, format 'adapter:target' (e.g. 'discord:123456789')."
                    },
                    "active_start_hour": {
                        "type": "integer",
                        "description": "For 'create': optional start of active window (0-23, 24h format)."
                    },
                    "active_end_hour": {
                        "type": "integer",
                        "description": "For 'create': optional end of active window (0-23, 24h format)."
                    },
                    "delete_id": {
                        "type": "string",
                        "description": "For 'delete': the ID of the heartbeat to remove."
                    }
                },
                "required": ["action"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action.as_str() {
            "create" => self.create(args).await,
            "list" => self.list().await,
            "delete" => self.delete(args).await,
            other => Ok(HeartbeatOutput {
                success: false,
                message: format!("Unknown action '{other}'. Use 'create', 'list', or 'delete'."),
                heartbeats: None,
            }),
        }
    }
}

impl HeartbeatTool {
    async fn create(&self, args: HeartbeatArgs) -> Result<HeartbeatOutput, HeartbeatError> {
        let id = args.id.ok_or_else(|| HeartbeatError("'id' is required for create".into()))?;
        let prompt = args
            .prompt
            .ok_or_else(|| HeartbeatError("'prompt' is required for create".into()))?;
        let interval_secs = args
            .interval_secs
            .ok_or_else(|| HeartbeatError("'interval_secs' is required for create".into()))?;
        let delivery_target = args
            .delivery_target
            .ok_or_else(|| HeartbeatError("'delivery_target' is required for create".into()))?;

        let active_hours = match (args.active_start_hour, args.active_end_hour) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        };

        let config = HeartbeatConfig {
            id: id.clone(),
            prompt: prompt.clone(),
            interval_secs,
            delivery_target: delivery_target.clone(),
            active_hours,
            enabled: true,
        };

        // Persist to database
        self.store
            .save(&config)
            .await
            .map_err(|error| HeartbeatError(format!("failed to save: {error}")))?;

        // Register with the running scheduler so it starts immediately
        self.scheduler
            .register(config)
            .await
            .map_err(|error| HeartbeatError(format!("failed to register: {error}")))?;

        let interval_desc = format_interval(interval_secs);
        let mut message = format!("Heartbeat '{id}' created. Runs {interval_desc}.");
        if let Some((start, end)) = active_hours {
            message.push_str(&format!(" Active {start:02}:00-{end:02}:00."));
        }

        tracing::info!(heartbeat_id = %id, %interval_secs, %delivery_target, "heartbeat created via tool");

        Ok(HeartbeatOutput {
            success: true,
            message,
            heartbeats: None,
        })
    }

    async fn list(&self) -> Result<HeartbeatOutput, HeartbeatError> {
        let configs = self
            .store
            .load_all()
            .await
            .map_err(|error| HeartbeatError(format!("failed to list: {error}")))?;

        let entries: Vec<HeartbeatEntry> = configs
            .into_iter()
            .map(|config| HeartbeatEntry {
                id: config.id,
                prompt: config.prompt,
                interval_secs: config.interval_secs,
                delivery_target: config.delivery_target,
                active_hours: config.active_hours.map(|(s, e)| format!("{s:02}:00-{e:02}:00")),
            })
            .collect();

        let count = entries.len();
        Ok(HeartbeatOutput {
            success: true,
            message: format!("{count} active heartbeat(s)."),
            heartbeats: Some(entries),
        })
    }

    async fn delete(&self, args: HeartbeatArgs) -> Result<HeartbeatOutput, HeartbeatError> {
        let id = args
            .delete_id
            .or(args.id)
            .ok_or_else(|| HeartbeatError("'delete_id' or 'id' is required for delete".into()))?;

        self.store
            .delete(&id)
            .await
            .map_err(|error| HeartbeatError(format!("failed to delete: {error}")))?;

        tracing::info!(heartbeat_id = %id, "heartbeat deleted via tool");

        Ok(HeartbeatOutput {
            success: true,
            message: format!("Heartbeat '{id}' deleted."),
            heartbeats: None,
        })
    }
}

fn format_interval(secs: u64) -> String {
    if secs % 86400 == 0 {
        let days = secs / 86400;
        if days == 1 {
            "every day".into()
        } else {
            format!("every {days} days")
        }
    } else if secs % 3600 == 0 {
        let hours = secs / 3600;
        if hours == 1 {
            "every hour".into()
        } else {
            format!("every {hours} hours")
        }
    } else if secs % 60 == 0 {
        let minutes = secs / 60;
        if minutes == 1 {
            "every minute".into()
        } else {
            format!("every {minutes} minutes")
        }
    } else {
        format!("every {secs} seconds")
    }
}
