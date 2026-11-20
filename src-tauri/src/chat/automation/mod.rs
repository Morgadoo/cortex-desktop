//! Work that runs without a prompt: agents and their organisation, the agent market, scheduled tasks, workflows, skills and plugins, the artifact lifecycle and evals.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod agent_market;
pub mod agents;
pub mod evals;
pub mod lifecycle;
pub mod org;
pub mod plugins;
pub mod skill_catalog;
pub mod skills;
pub mod tasks;
pub mod workflows;
