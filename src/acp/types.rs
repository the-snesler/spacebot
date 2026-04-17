//! UI-facing ACP update types.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};

fn stringify_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn content_block_text(content: &acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text.clone(),
        acp::ContentBlock::ResourceLink(link) => link.uri.clone(),
        acp::ContentBlock::Image(image) => image
            .uri
            .clone()
            .unwrap_or_else(|| format!("<image:{}>", image.mime_type)),
        acp::ContentBlock::Audio(audio) => format!("<audio:{}>", audio.mime_type),
        acp::ContentBlock::Resource(resource) => {
            serde_json::to_string(&resource.resource).unwrap_or_else(|_| "<resource>".into())
        }
        _ => "<content>".into(),
    }
}

fn tool_kind_name(kind: acp::ToolKind) -> String {
    match kind {
        acp::ToolKind::Read => "read",
        acp::ToolKind::Edit => "edit",
        acp::ToolKind::Delete => "delete",
        acp::ToolKind::Move => "move",
        acp::ToolKind::Search => "search",
        acp::ToolKind::Execute => "execute",
        acp::ToolKind::Think => "think",
        acp::ToolKind::Fetch => "fetch",
        acp::ToolKind::SwitchMode => "switch_mode",
        acp::ToolKind::Other => "other",
        _ => "other",
    }
    .to_string()
}

/// ACP tool execution state for the UI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl From<acp::ToolCallStatus> for AcpToolStatus {
    fn from(value: acp::ToolCallStatus) -> Self {
        match value {
            acp::ToolCallStatus::Pending => Self::Pending,
            acp::ToolCallStatus::InProgress => Self::InProgress,
            acp::ToolCallStatus::Completed => Self::Completed,
            acp::ToolCallStatus::Failed => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// ACP plan entry projection for the UI.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
pub struct AcpPlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

impl From<&acp::PlanEntry> for AcpPlanEntry {
    fn from(value: &acp::PlanEntry) -> Self {
        Self {
            content: value.content.clone(),
            priority: match value.priority {
                acp::PlanEntryPriority::High => "high",
                acp::PlanEntryPriority::Medium => "medium",
                acp::PlanEntryPriority::Low => "low",
                _ => "medium",
            }
            .to_string(),
            status: match value.status {
                acp::PlanEntryStatus::Pending => "pending",
                acp::PlanEntryStatus::InProgress => "in_progress",
                acp::PlanEntryStatus::Completed => "completed",
                _ => "pending",
            }
            .to_string(),
        }
    }
}

/// UI-facing projection of ACP session updates.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpUpdate {
    AgentMessage {
        id: String,
        text: String,
    },
    UserMessage {
        id: String,
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        title: String,
        input: Option<String>,
    },
    ToolCallUpdate {
        id: String,
        status: AcpToolStatus,
        output: Option<String>,
        error: Option<String>,
    },
    Plan {
        entries: Vec<AcpPlanEntry>,
    },
    StepFinish {
        stop_reason: String,
    },
}

impl AcpUpdate {
    pub fn from_session_update(update: &acp::SessionUpdate) -> Option<Self> {
        match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => Some(Self::AgentMessage {
                id: "agent".into(),
                text: content_block_text(&chunk.content),
            }),
            acp::SessionUpdate::UserMessageChunk(chunk) => Some(Self::UserMessage {
                id: "user".into(),
                text: content_block_text(&chunk.content),
            }),
            acp::SessionUpdate::AgentThoughtChunk(chunk) => Some(Self::AgentMessage {
                id: "thought".into(),
                text: content_block_text(&chunk.content),
            }),
            acp::SessionUpdate::ToolCall(tool_call) => Some(Self::ToolCall {
                id: tool_call.tool_call_id.to_string(),
                name: tool_kind_name(tool_call.kind),
                title: tool_call.title.clone(),
                input: tool_call.raw_input.as_ref().map(stringify_json),
            }),
            acp::SessionUpdate::ToolCallUpdate(update) => {
                let output = update.fields.raw_output.as_ref().map(stringify_json);
                let status = update.fields.status.unwrap_or(acp::ToolCallStatus::Pending);
                let (output, error) = if matches!(status, acp::ToolCallStatus::Failed) {
                    (None, output)
                } else {
                    (output, None)
                };
                Some(Self::ToolCallUpdate {
                    id: update.tool_call_id.to_string(),
                    status: status.into(),
                    output,
                    error,
                })
            }
            acp::SessionUpdate::Plan(plan) => Some(Self::Plan {
                entries: plan.entries.iter().map(AcpPlanEntry::from).collect(),
            }),
            acp::SessionUpdate::AvailableCommandsUpdate(_)
            | acp::SessionUpdate::CurrentModeUpdate(_)
            | _ => None,
        }
    }

    pub fn step_finish(stop_reason: acp::StopReason) -> Self {
        Self::StepFinish {
            stop_reason: match stop_reason {
                acp::StopReason::EndTurn => "end_turn",
                acp::StopReason::MaxTokens => "max_tokens",
                acp::StopReason::MaxTurnRequests => "max_turn_requests",
                acp::StopReason::Refusal => "refusal",
                acp::StopReason::Cancelled => "cancelled",
                _ => "unknown",
            }
            .to_string(),
        }
    }

    pub fn agent_text(&self) -> Option<&str> {
        match self {
            Self::AgentMessage { text, .. } => Some(text),
            _ => None,
        }
    }
}
