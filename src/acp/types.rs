//! ACP worker types: result struct, part enum, and helpers.

use crate::conversation::worker_transcript::{ActionContent, TranscriptStep};
use serde::{Deserialize, Serialize};

/// Result of an ACP worker run.
pub struct AcpWorkerResult {
    pub result_text: String,
    pub transcript: Vec<TranscriptStep>,
    pub tool_calls: i64,
}

/// A live content part from an ACP session, sent to the frontend for real-time
/// transcript rendering. Mirrors `OpenCodePart` semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpPart {
    /// Agent text output.
    Text { text: String },
    /// Agent thought/reasoning (not shown in final output).
    Thought { text: String },
    /// A tool call has started.
    ToolStarted {
        id: String,
        name: String,
    },
    /// A tool call has completed (or failed).
    ToolCompleted {
        id: String,
        name: String,
        result: String,
    },
}

impl AcpPart {
    /// Get a stable identifier for deduplication.
    pub fn id(&self) -> Option<&str> {
        match self {
            AcpPart::ToolStarted { id, .. } | AcpPart::ToolCompleted { id, .. } => Some(id),
            AcpPart::Text { .. } | AcpPart::Thought { .. } => None,
        }
    }
}

/// Convert accumulated ACP parts into transcript steps.
pub fn convert_acp_parts(parts: &[AcpPart]) -> Vec<TranscriptStep> {
    let mut steps = Vec::new();
    let mut current_action = Vec::new();

    for part in parts {
        match part {
            AcpPart::Text { text } => {
                // Flush any pending tool calls into an action step before text.
                if !current_action.is_empty() {
                    steps.push(TranscriptStep::Action {
                        content: std::mem::take(&mut current_action),
                    });
                }
                current_action.push(ActionContent::Text { text: text.clone() });
            }
            AcpPart::Thought { .. } => {
                // Thoughts are not included in the transcript.
            }
            AcpPart::ToolStarted { id, name } => {
                current_action.push(ActionContent::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args: String::new(),
                });
            }
            AcpPart::ToolCompleted { id, name, result } => {
                // Flush the current action (contains the ToolCall for this tool).
                if !current_action.is_empty() {
                    steps.push(TranscriptStep::Action {
                        content: std::mem::take(&mut current_action),
                    });
                }
                steps.push(TranscriptStep::ToolResult {
                    call_id: id.clone(),
                    name: name.clone(),
                    text: result.clone(),
                });
            }
        }
    }

    // Flush remaining action content.
    if !current_action.is_empty() {
        steps.push(TranscriptStep::Action {
            content: current_action,
        });
    }

    steps
}
