//! Conversations, projects and instructions on disk, under `<config>/chat/`.
//!
//! A conversation stores rig's [`Message`] history verbatim, so the next turn sends exactly what
//! the model saw. The page never reads that format: [`ConversationView`] flattens it into turns of
//! text, reasoning and tool steps.

use crate::support::error::StoreError;
use crate::support::settings_file::{self, Storage};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rig_core::completion::message::{AssistantContent, Message, ToolResultContent, UserContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The conversation file format. Raise it when a change needs [`migrate`] to convert older files;
/// files record the format they were written in, so a migration knows what it is reading.
pub const SCHEMA: u32 = 2;

pub fn chat_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("chat")
}

fn conversations_dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("conversations")
}

/// Ids name files, so only a uuid-like alphabet is accepted.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn conversation_path(config_dir: &Path, id: &str) -> Result<PathBuf, StoreError> {
    if !valid_id(id) {
        return Err(StoreError::InvalidId {
            kind: "conversation",
            id: id.to_string(),
        });
    }
    Ok(conversations_dir(config_dir).join(format!("{id}.json")))
}

/// Writes through a temporary file, so a crash never leaves half a conversation. Sealed when
/// encryption at rest is on.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    settings_file::write_json(path, Storage::Sealed, value)
}

/// Reads a sealed-or-plain JSON file; `None` when it is absent or (logged) unreadable.
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    settings_file::read_json(path, Storage::Sealed).unwrap_or_else(|e| {
        log::warn!("ignoring {e}");
        None
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    /// The format this file was written in; see [`SCHEMA`].
    #[serde(default)]
    pub schema: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Code sessions: the folder the assistant works in.
    #[serde(default)]
    pub workdir: Option<String>,
    /// The agent the conversation runs as.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The scheduled task that started it.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Finished while the user was elsewhere and not opened since.
    #[serde(default)]
    pub unread: bool,
    /// Kept out of the history and deleted on the next start.
    #[serde(default)]
    pub temporary: bool,
    /// The parts of the system prompt the latest turn sent and their sizes, for the context view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_parts: Vec<cortex_kernel::prompt::Part>,
    /// Hidden from the main list; still searchable and openable.
    #[serde(default)]
    pub archived: bool,
    /// Free-form labels, lowercase, for finding conversations again.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Branch point saved before branching became a tree; moved into `branches` on first use.
    #[serde(default, skip_serializing)]
    pub versions: Option<Versions>,
    /// Every point where the conversation branched, ordered by position. Branch points inside a
    /// stored branch travel with it and come back when it is shown again.
    #[serde(default)]
    pub branches: Vec<Versions>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub messages: Vec<Message>,
    /// When each user turn was sent and each reply finished, by message index, for the page's
    /// timestamps. Missing entries are fine.
    #[serde(default)]
    pub times: Vec<(usize, DateTime<Utc>)>,
    /// The user's rating of each reply, by the index of its last message.
    #[serde(default)]
    pub ratings: Vec<(usize, i8)>,
    /// What the user said was wrong with a reply they rated down, by the same index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rating_reasons: Vec<(usize, String)>,
    /// Messages the user bookmarked, by the index of the turn's last message, with a snippet for
    /// the bookmarks list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<(usize, String)>,
    /// How long the model thought before each reply message, in milliseconds, by message index.
    #[serde(default)]
    pub thought: Vec<(usize, u32)>,
    /// Notices shown inline before the message at that index.
    #[serde(default)]
    pub notices: Vec<(usize, String)>,
    /// Tools loaded on demand with `find_tools`, kept for the rest of the conversation.
    #[serde(default)]
    pub loaded_tools: Vec<String>,
    /// Model settings for this conversation only (temperature, reply length, instructions).
    #[serde(default, skip_serializing_if = "ChatParams::is_empty")]
    pub params: ChatParams,
    /// The note standing in for compacted turns.
    #[serde(default)]
    pub summary: Option<cortex_kernel::context::Summary>,
    #[serde(default)]
    pub usage: cortex_kernel::context::UsageTotals,
}

impl Conversation {
    pub fn new(id: String, project_id: Option<String>, model: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            schema: SCHEMA,
            id,
            title: String::new(),
            project_id,
            model,
            pinned: false,
            workdir: None,
            agent_id: None,
            task_id: None,
            unread: false,
            temporary: false,
            prompt_parts: Vec::new(),
            archived: false,
            tags: Vec::new(),
            versions: None,
            branches: Vec::new(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            times: Vec::new(),
            thought: Vec::new(),
            notices: Vec::new(),
            loaded_tools: Vec::new(),
            params: ChatParams::default(),
            ratings: Vec::new(),
            rating_reasons: Vec::new(),
            bookmarks: Vec::new(),
            summary: None,
            usage: Default::default(),
        }
    }
}

/// Model settings for one conversation; unset ones follow the model's defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct ChatParams {
    /// 0 (focused) to 2 (inventive).
    #[serde(default)]
    pub temperature: Option<f64>,
    /// The longest reply, in tokens.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub max_tokens: Option<u64>,
    /// Nucleus sampling: only the likeliest words adding up to this share are considered (0–1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    /// Instructions for this conversation, added to the standing ones.
    #[serde(default)]
    pub instructions: String,
}

impl ChatParams {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Settings within what providers accept.
    pub fn clamped(self) -> Self {
        Self {
            temperature: self
                .temperature
                .filter(|t| t.is_finite())
                .map(|t| t.clamp(0.0, 2.0)),
            max_tokens: self
                .max_tokens
                .filter(|&n| n > 0)
                .map(|n| n.clamp(16, 128_000)),
            top_p: self
                .top_p
                .filter(|p| p.is_finite() && *p > 0.0)
                .map(|p| p.min(1.0)),
            instructions: crate::support::text::clip(self.instructions.trim(), 8_000),
        }
    }

    /// This chat's settings, with the app-wide defaults filling what it leaves unset.
    pub fn or_defaults(&self, defaults: &ChatParams) -> ChatParams {
        ChatParams {
            temperature: self.temperature.or(defaults.temperature),
            max_tokens: self.max_tokens.or(defaults.max_tokens),
            top_p: self.top_p.or(defaults.top_p),
            instructions: self.instructions.clone(),
        }
    }
}

fn defaults_path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("chat-defaults.json")
}

/// Model settings every chat starts from (Open WebUI's global model defaults).
pub fn chat_defaults(config_dir: &Path) -> ChatParams {
    read_json(&defaults_path(config_dir)).unwrap_or_default()
}

pub fn save_chat_defaults(config_dir: &Path, params: ChatParams) -> Result<ChatParams, String> {
    let params = ChatParams {
        instructions: String::new(),
        ..params.clamped()
    };
    write_json(&defaults_path(config_dir), &params)?;
    Ok(params)
}

pub fn load_conversation(config_dir: &Path, id: &str) -> Result<Conversation, StoreError> {
    read_json(&conversation_path(config_dir, id)?).ok_or_else(|| StoreError::NotFound {
        kind: "conversation",
        id: id.to_string(),
    })
}

pub fn save_conversation(config_dir: &Path, conversation: &Conversation) -> Result<(), StoreError> {
    let path = conversation_path(config_dir, &conversation.id)?;
    if !conversation.temporary {
        crate::chat::backup::before_overwrite(config_dir, &path);
    }
    if conversation.schema < SCHEMA {
        let mut current = conversation.clone();
        current.schema = SCHEMA;
        return write_json(&path, &current);
    }
    write_json(&path, conversation)
}

/// Deletes the temporary conversations a previous session left behind.
pub fn purge_temporary(config_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(conversations_dir(config_dir)) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        if read_json::<Conversation>(&entry.path()).is_some_and(|c| c.temporary) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Deleting moves the conversation to the trash, where it stays for 30 days.
pub fn delete_conversation(config_dir: &Path, id: &str) -> Result<(), String> {
    crate::chat::backup::trash(config_dir, id)
}

/// A sidebar row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct ConversationSummary {
    pub archived: bool,
    pub tags: Vec<String>,
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub workdir: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub unread: bool,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Tokens used (in and out) and their cost, for budgets and the agent pages.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub tokens: u64,
    #[serde(default)]
    pub cost: f64,
    /// The model the conversation last used, for usage by model.
    #[serde(default)]
    pub model: Option<String>,
    /// The start of the latest reply (or message), for the sidebar's hover preview.
    #[serde(default)]
    pub preview: String,
    /// Bookmarked messages: turn index and snippet.
    #[serde(default)]
    pub bookmarks: Vec<(usize, String)>,
}

/// The sidebar rows, from the index (see [`super::index`]), so a long history stays fast.
pub fn list_conversations(config_dir: &Path) -> Vec<ConversationSummary> {
    crate::chat::index::list(config_dir)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Instructions added to every conversation in the project.
    #[serde(default)]
    pub instructions: String,
    /// A tone preset for the project's conversations; the user's own when empty.
    #[serde(default)]
    pub tone: String,
    /// An emoji shown beside the name.
    #[serde(default)]
    pub icon: String,
    /// The library holding the project's files.
    #[serde(default)]
    pub library_id: Option<String>,
    /// New conversations see short summaries of the project's other conversations.
    #[serde(default)]
    pub share_chats: bool,
}

fn projects_path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("projects.json")
}

pub fn load_projects(config_dir: &Path) -> Vec<Project> {
    read_json(&projects_path(config_dir)).unwrap_or_default()
}

pub fn save_projects(config_dir: &Path, projects: &[Project]) -> Result<(), String> {
    write_json(&projects_path(config_dir), &projects).map_err(String::from)
}

/// The user's standing instructions: a tone preset and free text.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct Instructions {
    pub tone: String,
    pub custom: String,
    /// How the assistant should call the user.
    pub name: String,
}

fn instructions_path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("instructions.json")
}

pub fn load_instructions(config_dir: &Path) -> Instructions {
    read_json(&instructions_path(config_dir)).unwrap_or_default()
}

pub fn save_instructions(config_dir: &Path, instructions: &Instructions) -> Result<(), String> {
    crate::chat::versions::keep(
        config_dir,
        "instructions",
        "user",
        &load_instructions(config_dir),
        instructions,
    )?;
    write_json(&instructions_path(config_dir), instructions).map_err(String::from)
}

mod editing;
mod rendering;
mod views;
pub use editing::*;
pub use rendering::*;
pub use views::*;

#[cfg(test)]
mod tests;
