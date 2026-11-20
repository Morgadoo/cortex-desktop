//! The models a turn can use: providers and their catalogues, routing, the built-in runtime, coding-agent back ends, and the arena.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod acp;
pub mod agent_cli;
pub mod arena;
pub mod batched_qwen2;
pub mod connect;
pub mod local_models;
pub mod privacy;
pub mod providers;
pub mod router;
