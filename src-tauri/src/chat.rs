//! Native, multi-provider chat built on [rig](https://rig.rs).
//!
//! The main window is a bundled page (`webview/pages/app`), and every model call happens here in
//! Rust: the page sends a prompt with [`commands::chat_send`], and the reply streams back as
//! [`engine::ChatEvent`]s on the `chat://event` channel. Conversations are rig's own message
//! history, saved as JSON, so a reloaded conversation is exactly what the model saw.
//!
//! The modules are grouped by what they are for. From the bottom up:
//!
//! - [`storage`]: conversations, their index, backups, versions, sync, encryption, secrets
//! - [`models`]: providers, routing, the built-in runtime, coding-agent back ends
//! - [`grounding`]: memories, knowledge, libraries, embeddings, search, reading the web
//! - [`toolkit`]: the code sandbox, written and custom tools, computer use, coding, canvases
//! - [`multimedia`]: images, speech, video, recording
//! - [`turn`]: one turn: the engine, tool dispatch, built-ins, sub-agents, context, the prompt
//! - [`automation`]: agents, scheduled tasks, workflows, skills and plugins, evals
//! - [`integrations`], [`cli`], [`ui`], [`commands`]: the ways in: other apps, the terminal,
//!   the window
//!
//! Every module is also re-exported here by its own name (`crate::chat::store`), which is how
//! the code refers to it.

pub mod commands;
pub mod storage;
pub use storage::{
    at_rest, audit, coding_import, import, outputs, recent_files, secrets, store, sync, uploads,
    vault, versions,
};
use storage::{backup, index};
pub mod models;
pub use models::{acp, agent_cli, arena, batched_qwen2, local_models, providers, router};
pub mod grounding;
pub use grounding::{
    calendar, embeddings, extraction, knowledge, learning, libraries, memory, reader, retrieval,
    search, search_all,
};
use grounding::{favicon, preview};
pub mod toolkit;
pub use toolkit::{
    canvas, code, computer, custom_tools, openapi, proposals, sandbox, script_tools, sheets,
    terminal, webpage, workbench,
};
pub mod multimedia;
pub use multimedia::{animation, audio, hex, media, recorder, video, videogen};
pub mod automation;
pub use automation::{
    agent_market, agents, evals, lifecycle, org, plugins, skill_catalog, skills, tasks, workflows,
};
pub mod integrations;
pub use integrations::{api_server, discord, links, share, slack, telegram};
pub mod cli;
pub use cli::{config_cli, headless, terminal_chat};
pub mod ui;
pub use ui::{emoji_pack, ui_language};
pub mod turn;
use turn::deferred;
pub use turn::{
    builtins, context, engine, functions, hooks, host, privacy, prompt, subagents, tools, traces,
};
mod state;
pub use state::ChatState;

/// The account every account-scoped subsystem (connectors, secrets, skills) runs under. Proton
/// accounts are gone; one local profile takes their place.
pub const LOCAL_ACCOUNT: &str = "local";

/// Event channel the page listens on.
pub const EVENT: &str = "chat://event";
