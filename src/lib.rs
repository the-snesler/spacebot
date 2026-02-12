//! Spacebot: A Rust agentic system where every LLM process has a dedicated role.

pub mod agent;
pub mod config;
pub mod conversation;
pub mod db;
pub mod error;
pub mod heartbeat;
pub mod hooks;
pub mod identity;
pub mod llm;
pub mod memory;
pub mod messaging;
pub mod secrets;
pub mod settings;
pub mod tools;

pub use error::{Error, Result};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Channel identifier type.
pub type ChannelId = Arc<str>;

/// Worker identifier type.
pub type WorkerId = uuid::Uuid;

/// Branch identifier type.
pub type BranchId = uuid::Uuid;

/// Process identifier type (union of channel, worker, branch IDs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessId {
    Channel(ChannelId),
    Worker(WorkerId),
    Branch(BranchId),
}

impl std::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessId::Channel(id) => write!(f, "channel:{}", id),
            ProcessId::Worker(id) => write!(f, "worker:{}", id),
            ProcessId::Branch(id) => write!(f, "branch:{}", id),
        }
    }
}

/// Process types in the system.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessType {
    Channel,
    Branch,
    Worker,
    Compactor,
    Cortex,
}

impl std::fmt::Display for ProcessType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessType::Channel => write!(f, "channel"),
            ProcessType::Branch => write!(f, "branch"),
            ProcessType::Worker => write!(f, "worker"),
            ProcessType::Compactor => write!(f, "compactor"),
            ProcessType::Cortex => write!(f, "cortex"),
        }
    }
}

/// Events sent between processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEvent {
    BranchResult {
        branch_id: BranchId,
        channel_id: ChannelId,
        conclusion: String,
    },
    WorkerStatus {
        worker_id: WorkerId,
        channel_id: Option<ChannelId>,
        status: String,
    },
    WorkerComplete {
        worker_id: WorkerId,
        channel_id: Option<ChannelId>,
        result: String,
        notify: bool,
    },
    ToolStarted {
        process_id: ProcessId,
        tool_name: String,
    },
    ToolCompleted {
        process_id: ProcessId,
        tool_name: String,
        result: String,
    },
    MemorySaved {
        memory_id: String,
        channel_id: Option<ChannelId>,
    },
    CompactionTriggered {
        channel_id: ChannelId,
        threshold_reached: f32,
    },
    StatusUpdate {
        process_id: ProcessId,
        status: String,
    },
}

/// Shared dependency bundle for agents.
#[derive(Clone)]
pub struct AgentDeps {
    pub memory_store: Arc<memory::MemoryStore>,
    pub llm_manager: Arc<llm::LlmManager>,
    pub tool_server: tools::ToolServerHandle,
    pub event_tx: tokio::sync::mpsc::Sender<ProcessEvent>,
}

/// Inbound message from any messaging platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub source: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub content: MessageContent,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Message content variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageContent {
    Text(String),
    Media {
        text: Option<String>,
        attachments: Vec<Attachment>,
    },
}

impl std::fmt::Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageContent::Text(text) => write!(f, "{}", text),
            MessageContent::Media { text, .. } => {
                if let Some(t) = text {
                    write!(f, "{}", t)
                } else {
                    write!(f, "[media]")
                }
            }
        }
    }
}

/// File attachment metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub url: String,
    pub size_bytes: Option<u64>,
}

/// Outbound response to messaging platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundResponse {
    Text(String),
    StreamStart,
    StreamChunk(String),
    StreamEnd,
}

/// Status updates for messaging platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusUpdate {
    Thinking,
    ToolStarted { tool_name: String },
    ToolCompleted { tool_name: String },
    BranchStarted { branch_id: BranchId },
    WorkerStarted { worker_id: WorkerId, task: String },
    WorkerCompleted { worker_id: WorkerId, result: String },
}
