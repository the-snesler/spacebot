//! SpacebotHook: Prompt hook for channels, branches, and workers.

use crate::error::Result;
use crate::{ProcessEvent, ProcessId, ProcessType};
use tokio::sync::mpsc;

/// Hook for observing agent behavior and sending events.
#[derive(Clone)]
pub struct SpacebotHook {
    process_id: ProcessId,
    process_type: ProcessType,
    event_tx: mpsc::Sender<ProcessEvent>,
}

/// Actions the hook can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    /// Continue processing normally.
    Continue,
    /// Skip this turn (for rate limiting, etc.).
    Skip,
    /// Terminate the process (for cancellation, budget exceeded, etc.).
    Terminate,
}

impl SpacebotHook {
    /// Create a new hook.
    pub fn new(
        process_id: ProcessId,
        process_type: ProcessType,
        event_tx: mpsc::Sender<ProcessEvent>,
    ) -> Self {
        Self {
            process_id,
            process_type,
            event_tx,
        }
    }

    /// Called when a tool is about to be called.
    pub fn on_tool_call(&self, tool_name: &str) -> HookAction {
        // Send event without blocking
        let event = ProcessEvent::ToolStarted {
            process_id: self.process_id.clone(),
            tool_name: tool_name.to_string(),
        };
        let _ = self.event_tx.try_send(event);

        HookAction::Continue
    }

    /// Called when a tool completes.
    pub fn on_tool_result(&self, tool_name: &str, result: &str) -> HookAction {
        // Scan for potential leaks in tool output
        if let Some(leak) = self.scan_for_leaks(result) {
            tracing::warn!(%leak, "potential secret leak detected in tool output");
            // Return the result but log the warning
        }

        let event = ProcessEvent::ToolCompleted {
            process_id: self.process_id.clone(),
            tool_name: tool_name.to_string(),
            result: result.to_string(),
        };
        let _ = self.event_tx.try_send(event);

        HookAction::Continue
    }

    /// Called on each completion response.
    pub fn on_completion_response(&self, iteration: usize, content: &str) -> HookAction {
        // Tool nudging: if first 2 iterations have no tool calls, prompt to use tools
        if iteration < 2 && !content.contains("tool") {
            tracing::debug!("response without tool calls detected, nudging");
        }

        HookAction::Continue
    }

    /// Send a status update event.
    pub fn send_status(&self, status: impl Into<String>) {
        let event = ProcessEvent::StatusUpdate {
            process_id: self.process_id.clone(),
            status: status.into(),
        };
        let _ = self.event_tx.try_send(event);
    }

    /// Scan content for potential secret leaks.
    fn scan_for_leaks(&self, content: &str) -> Option<String> {
        use regex::Regex;
        use std::sync::LazyLock;

        static LEAK_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
            vec![
                Regex::new(r"sk-[a-zA-Z0-9]{48}").expect("hardcoded regex"),
                Regex::new(r"-----BEGIN.*PRIVATE KEY-----").expect("hardcoded regex"),
                Regex::new(r"ghp_[a-zA-Z0-9]{36}").expect("hardcoded regex"),
                Regex::new(r"AIza[0-9A-Za-z_-]{35}").expect("hardcoded regex"),
            ]
        });

        for pattern in LEAK_PATTERNS.iter() {
            if let Some(matched) = pattern.find(content) {
                return Some(matched.as_str().to_string());
            }
        }

        None
    }
}
