//! The agent kernel's functional core: the decisions the agent loop makes, with no window, no
//! files and no network. The app's loop (`chat/engine.rs`) calls these and does the rest.
//!
//! - [`agent_loop`]: the loop's control flow: steps, the last step, guidance, cancelling.
//! - [`approval`]: approval modes, hard-to-undo calls, how a change meets the mode.
//! - [`tool`]: a tool as offered to the model, and what a call returned.
//! - [`context`]: what a request carries, compaction, the context view.
//! - [`prompt`]: the system prompt as named, budgeted sections.
//! - [`privacy`]: personal data swapped for placeholders before a cloud request.
//! - [`evals`]: what a case expects, grading, comparing runs.
//! - [`lifecycle`]: what to retire, archive or demote, and each artifact's score.
//! - [`diff`]: what changed between two versions.
//!
//! Beside it: `cortex-retrieval` (ranking), `cortex-traces` (the record), `cortex-sandbox` (the
//! WASM sandbox).

pub mod agent_loop;
pub mod approval;
pub mod context;
pub mod diff;
pub mod evals;
pub mod lifecycle;
pub mod privacy;
pub mod prompt;
pub mod tool;
