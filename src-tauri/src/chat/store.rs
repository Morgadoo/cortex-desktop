//! Conversations, projects and instructions on disk, under `<config>/chat/`.
//!
//! A conversation stores rig's [`Message`] history verbatim, so the next turn sends exactly what
//! the model saw. The page never reads that format: [`ConversationView`] flattens it into turns of
//! text, reasoning and tool steps.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rig_core::completion::message::{
    AssistantContent, Message, ToolResultContent, UserContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

fn conversation_path(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("invalid conversation id `{id}`"));
    }
    Ok(conversations_dir(config_dir).join(format!("{id}.json")))
}

/// Writes through a temporary file, so a crash never leaves half a conversation.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {parent:?}: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("could not write {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {path:?}: {e}"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(e) => {
            log::warn!("ignoring unreadable {path:?}: {e}");
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
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
    /// The note standing in for compacted turns.
    #[serde(default)]
    pub summary: Option<super::context::Summary>,
    #[serde(default)]
    pub usage: super::context::UsageTotals,
}

impl Conversation {
    pub fn new(id: String, project_id: Option<String>, model: Option<String>) -> Self {
        let now = Utc::now();
        Self {
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
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            times: Vec::new(),
            ratings: Vec::new(),
            summary: None,
            usage: Default::default(),
        }
    }
}

pub fn load_conversation(config_dir: &Path, id: &str) -> Result<Conversation, String> {
    read_json(&conversation_path(config_dir, id)?).ok_or_else(|| format!("no conversation `{id}`"))
}

pub fn save_conversation(config_dir: &Path, conversation: &Conversation) -> Result<(), String> {
    write_json(&conversation_path(config_dir, &conversation.id)?, conversation)
}

/// Deletes the temporary conversations a previous session left behind.
pub fn purge_temporary(config_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(conversations_dir(config_dir)) else { return };
    for entry in entries.filter_map(Result::ok) {
        if read_json::<Conversation>(&entry.path()).is_some_and(|c| c.temporary) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub fn delete_conversation(config_dir: &Path, id: &str) -> Result<(), String> {
    match std::fs::remove_file(conversation_path(config_dir, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// A sidebar row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub workdir: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub unread: bool,
    pub pinned: bool,
    pub updated_at: DateTime<Utc>,
}

pub fn list_conversations(config_dir: &Path) -> Vec<ConversationSummary> {
    let Ok(entries) = std::fs::read_dir(conversations_dir(config_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<ConversationSummary> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read_json::<Conversation>(&e.path()))
        .filter(|c| !c.temporary)
        .map(|c| ConversationSummary {
            title: if c.title.is_empty() { "New chat".into() } else { c.title },
            id: c.id,
            project_id: c.project_id,
            workdir: c.workdir,
            agent_id: c.agent_id,
            task_id: c.task_id,
            unread: c.unread,
            pinned: c.pinned,
            updated_at: c.updated_at,
        })
        .collect();
    out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Instructions added to every conversation in the project.
    #[serde(default)]
    pub instructions: String,
}

fn projects_path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("projects.json")
}

pub fn load_projects(config_dir: &Path) -> Vec<Project> {
    read_json(&projects_path(config_dir)).unwrap_or_default()
}

pub fn save_projects(config_dir: &Path, projects: &[Project]) -> Result<(), String> {
    write_json(&projects_path(config_dir), &projects)
}

/// The user's standing instructions: a tone preset and free text.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
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
    write_json(&instructions_path(config_dir), instructions)
}

/// One step of an assistant turn, as the page draws it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Part {
    Text { text: String },
    Reasoning { text: String },
    #[serde(rename_all = "camelCase")]
    Image { media_type: String, data: String },
    #[serde(rename_all = "camelCase")]
    Tool {
        call_id: String,
        name: String,
        arguments: Value,
        output: Option<String>,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    /// `user` or `assistant`.
    pub role: &'static str,
    pub parts: Vec<Part>,
    /// Index of the turn's last message, which ratings and timestamps key on.
    pub index: usize,
    pub time: Option<DateTime<Utc>>,
    pub rating: i8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationView {
    pub id: String,
    pub usage: super::context::UsageTotals,
    /// Index of the first turn requests still carry verbatim, when earlier ones were compacted.
    pub compacted_before: Option<usize>,
    pub title: String,
    pub project_id: Option<String>,
    pub workdir: Option<String>,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub pinned: bool,
    pub turns: Vec<Turn>,
}

fn result_text(content: &[ToolResultContent]) -> String {
    content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(t) => t.text.clone(),
            ToolResultContent::Json { value, .. } => value.to_string(),
            _ => "[non-text result]".to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Flattens rig's history into turns. Tool results, which rig carries as user messages, fold into
/// the assistant turn that made the call, so a turn reads: thought, tool, tool, answer.
pub fn turns(conversation: &Conversation) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    let time_of = |i: usize| conversation.times.iter().find(|(k, _)| *k == i).map(|(_, t)| *t);
    let rating_of = |i: usize| {
        conversation
            .ratings
            .iter()
            .find(|(k, _)| *k == i)
            .map_or(0, |(_, r)| *r)
    };
    for (index, message) in conversation.messages.iter().enumerate() {
        match message {
            Message::System { .. } => {}
            Message::User { content } => {
                let mut texts = Vec::new();
                for item in content {
                    match item {
                        UserContent::Text(t) => texts.push(Part::Text { text: t.text.clone() }),
                        UserContent::Image(image) => {
                            if let rig_core::completion::message::DocumentSourceKind::Base64(data) = &image.data {
                                use rig_core::completion::message::MimeType;
                                texts.push(Part::Image {
                                    media_type: image.media_type.as_ref().map_or("image/png", |m| m.to_mime_type()).to_string(),
                                    data: data.clone(),
                                });
                            }
                        }
                        UserContent::ToolResult(result) => {
                            let call = result.call.to_string();
                            let tool = out.last_mut().filter(|t| t.role == "assistant").and_then(|turn| {
                                turn.parts.iter_mut().find(|p| {
                                    matches!(p, Part::Tool { call_id, .. } if *call_id == call)
                                })
                            });
                            if let Some(Part::Tool { output, is_error, .. }) = tool {
                                *output = Some(result_text(&result.content));
                                *is_error = result.is_error;
                            }
                            if let Some(turn) = out.last_mut() {
                                turn.index = index;
                            }
                        }
                        _ => {}
                    }
                }
                if !texts.is_empty() {
                    out.push(Turn {
                        role: "user",
                        parts: texts,
                        index,
                        time: time_of(index),
                        rating: 0,
                    });
                }
            }
            Message::Assistant(assistant) => {
                let parts = assistant.content.iter().filter_map(|c| match c {
                    AssistantContent::Text(t) if !t.text.is_empty() => {
                        Some(Part::Text { text: t.text.clone() })
                    }
                    AssistantContent::Reasoning(r) if !r.text.is_empty() => {
                        Some(Part::Reasoning { text: r.text.clone() })
                    }
                    AssistantContent::ToolCall(call) => Some(Part::Tool {
                        call_id: call.id.to_string(),
                        name: call.function.name.to_string(),
                        arguments: Value::Object(call.function.arguments.clone()),
                        output: None,
                        is_error: false,
                    }),
                    _ => None,
                });
                match out.last_mut() {
                    Some(turn) if turn.role == "assistant" => {
                        turn.parts.extend(parts);
                        turn.index = index;
                    }
                    _ => out.push(Turn {
                        role: "assistant",
                        parts: parts.collect(),
                        index,
                        time: None,
                        rating: 0,
                    }),
                }
            }
        }
    }
    for turn in out.iter_mut().filter(|t| t.role == "assistant") {
        turn.time = time_of(turn.index);
        turn.rating = rating_of(turn.index);
    }
    out
}

pub fn view(conversation: &Conversation) -> ConversationView {
    ConversationView {
        id: conversation.id.clone(),
        usage: conversation.usage.clone(),
        compacted_before: conversation.summary.as_ref().map(|s| s.upto),
        title: conversation.title.clone(),
        project_id: conversation.project_id.clone(),
        workdir: conversation.workdir.clone(),
        agent_id: conversation.agent_id.clone(),
        model: conversation.model.clone(),
        pinned: conversation.pinned,
        turns: turns(conversation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::completion::message::{AssistantMessage, CallId, ToolName};

    fn tool_call_message(id: &str, name: &str) -> Message {
        Message::Assistant(AssistantMessage::new(vec![AssistantContent::tool_call(
            id,
            ToolName::new(name).unwrap(),
            serde_json::json!({ "path": "/tmp" }),
        )]))
    }

    #[test]
    fn ids_must_be_safe_file_names() {
        assert!(valid_id("8f2c-11aa_x"));
        assert!(!valid_id("../etc"));
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
    }

    #[test]
    fn conversations_round_trip_and_list_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut older = Conversation::new("a".into(), None, None);
        older.messages.push(Message::user("hi"));
        older.updated_at = Utc::now() - chrono::Duration::hours(1);
        save_conversation(dir.path(), &older).unwrap();
        let mut newer = Conversation::new("b".into(), None, None);
        newer.title = "Second".into();
        save_conversation(dir.path(), &newer).unwrap();

        let list = list_conversations(dir.path());
        assert_eq!(list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(list[1].title, "New chat");
        let loaded = load_conversation(dir.path(), "a").unwrap();
        assert_eq!(loaded.messages, older.messages);

        delete_conversation(dir.path(), "a").unwrap();
        assert_eq!(list_conversations(dir.path()).len(), 1);
    }

    #[test]
    fn tool_results_fold_into_the_assistant_turn() {
        let mut c = Conversation::new("c".into(), None, None);
        c.messages.push(Message::user("list files"));
        c.messages.push(tool_call_message("call_1", "filesystem__list"));
        c.messages.push(Message::tool_result(
            CallId::from_wire("call_1"),
            ToolName::new("filesystem__list").unwrap(),
            "a.txt",
        ));
        c.messages.push(Message::assistant("There is one file."));

        let turns = turns(&c);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].index, 3);
        match &turns[1].parts[0] {
            Part::Tool { name, output, .. } => {
                assert_eq!(name, "filesystem__list");
                assert_eq!(output.as_deref(), Some("a.txt"));
            }
            other => panic!("expected a tool part, got {other:?}"),
        }
        assert_eq!(turns[1].parts[1], Part::Text { text: "There is one file.".into() });
    }
}
