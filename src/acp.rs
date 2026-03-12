//! ACP (Agent Client Protocol) worker support.
//!
//! Provides a third worker type alongside builtin (Rig agent) and OpenCode
//! (HTTP/SSE subprocess). ACP workers communicate with any ACP-compatible
//! coding agent CLI (Claude Code, Codex, Gemini CLI, etc.) via bidirectional
//! JSON-RPC 2.0 over stdio.
//!
//! ## Thread Model
//!
//! The `agent-client-protocol` crate uses `#[async_trait(?Send)]` for both
//! `Client` and `Agent` traits. This means the entire ACP connection must
//! run on a single-threaded `LocalSet` — not directly on tokio's multi-threaded
//! runtime. Each ACP worker spawns a dedicated `std::thread` running a
//! single-threaded tokio runtime with a `LocalSet`. Communication between the
//! ACP thread and the main runtime uses `broadcast` (events) and `mpsc`
//! (follow-up input) channels.

pub mod client;
pub mod types;
pub mod worker;

pub use types::AcpPart;
pub use worker::AcpWorker;
