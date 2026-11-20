//! One turn of the assistant: the engine and its tool handlers, the host seam, tool dispatch and built-ins, deferred tools, sub-agents, hooks and functions, the privacy filter, context, the prompt and traces.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod builtins;
pub mod context;
pub(super) mod deferred;
pub mod engine;
pub mod functions;
pub mod headless_host;
pub mod hooks;
pub mod host;
pub mod prompt;
pub mod subagents;
pub mod tools;
