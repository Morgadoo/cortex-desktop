//! Command-line modes that run before the window: headless runs, terminal chat and configuration.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod config_cli;
pub mod headless;
pub mod terminal_chat;
