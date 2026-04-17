//! ACP subprocess integration for coding workers.

pub mod bridge;
pub mod client;
pub mod config;
pub mod types;
pub mod worker;

pub use bridge::{AcpCmd, AcpEvt, AcpPermissionOption, spawn_acp_bridge};
pub use config::{AcpConfig, AcpProfile, AcpProfileInfo, validate_acp_config};
pub use types::{AcpPlanEntry, AcpToolStatus, AcpUpdate};
pub use worker::AcpWorker;
