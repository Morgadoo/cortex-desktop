//! What every layer of the chat engine shares: the events a turn streams to the window, and the
//! names of the built-in tools.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod approval;
pub mod error;
pub mod events;
pub mod tool_names;
