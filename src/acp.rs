//! ACP (Agent Client Protocol) worker backend.
//!
//! Runs external coding agents (Claude Code, Codex, Gemini CLI, etc.) as
//! subprocess workers that communicate over JSON-RPC/stdio using the
//! `agent-client-protocol` crate.

pub mod client;
pub mod process;
pub mod worker;

pub use worker::{AcpWorker, AcpWorkerResult};
