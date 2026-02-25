//! Agent Client Protocol (ACP) integration for worker backends.
//!
//! ACP workers run external coding agents over stdio using the
//! `agent-client-protocol` crate. Spacebot acts as the ACP client side,
//! implementing filesystem + terminal capabilities required by coding agents.

pub mod worker;

pub use worker::{AcpWorker, AcpWorkerResult};
