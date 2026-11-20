//! Other programs reaching the assistant: Slack, Discord and Telegram, the local API server, share links and deep links.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod api_server;
pub mod discord;
pub mod links;
pub mod platform;
pub mod share;
pub mod slack;
pub mod telegram;
