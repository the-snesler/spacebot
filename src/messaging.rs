//! Messaging adapters (Discord, Telegram, Webhook).

pub mod traits;
pub mod manager;
pub mod discord;
pub mod telegram;
pub mod webhook;

pub use traits::Messaging;
pub use manager::MessagingManager;
