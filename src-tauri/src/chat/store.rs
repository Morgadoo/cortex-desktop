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
    pub summary: Option<super::context::Summary>,
    #[serde(default)]
    pub usage: super::context::UsageTotals,
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
pub struct ChatParams {
    /// 0 (focused) to 2 (inventive).
    #[serde(default)]
    pub temperature: Option<f64>,
    /// The longest reply, in tokens.
    #[serde(default)]
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
            temperature: self.temperature.filter(|t| t.is_finite()).map(|t| t.clamp(0.0, 2.0)),
            max_tokens: self.max_tokens.filter(|&n| n > 0).map(|n| n.clamp(16, 128_000)),
            top_p: self.top_p.filter(|p| p.is_finite() && *p > 0.0).map(|p| p.min(1.0)),
            instructions: self.instructions.trim().chars().take(8_000).collect(),
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
    let params = ChatParams { instructions: String::new(), ..params.clamped() };
    write_json(&defaults_path(config_dir), &params)?;
    Ok(params)
}

pub fn load_conversation(config_dir: &Path, id: &str) -> Result<Conversation, String> {
    read_json(&conversation_path(config_dir, id)?).ok_or_else(|| format!("no conversation `{id}`"))
}

pub fn save_conversation(config_dir: &Path, conversation: &Conversation) -> Result<(), String> {
    let path = conversation_path(config_dir, &conversation.id)?;
    if !conversation.temporary {
        super::backup::before_overwrite(config_dir, &path);
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
    let Ok(entries) = std::fs::read_dir(conversations_dir(config_dir)) else { return };
    for entry in entries.filter_map(Result::ok) {
        if read_json::<Conversation>(&entry.path()).is_some_and(|c| c.temporary) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Deleting moves the conversation to the trash, where it stays for 30 days.
pub fn delete_conversation(config_dir: &Path, id: &str) -> Result<(), String> {
    super::backup::trash(config_dir, id)
}

/// A sidebar row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    pub tokens: u64,
    #[serde(default)]
    pub cost: f64,
    /// The start of the latest reply (or message), for the sidebar's hover preview.
    #[serde(default)]
    pub preview: String,
    /// Bookmarked messages: turn index and snippet.
    #[serde(default)]
    pub bookmarks: Vec<(usize, String)>,
}

/// The sidebar rows, from the index (see [`super::index`]), so a long history stays fast.
pub fn list_conversations(config_dir: &Path) -> Vec<ConversationSummary> {
    super::index::list(config_dir)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
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
    /// Something the user should know about the reply, shown inline (Vibe 3.1.0): a backup
    /// model taking over, for example.
    Notice { text: String },
    Reasoning {
        text: String,
        /// How long the model thought, in milliseconds, when measured.
        #[serde(skip_serializing_if = "Option::is_none")]
        ms: Option<u32>,
    },
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
    pub archived: bool,
    pub params: ChatParams,
    /// Turn indexes the user bookmarked.
    pub bookmarks: Vec<usize>,
    pub turns: Vec<Turn>,
    /// The branch points along the showing path, each with its switcher position.
    pub versions: Vec<VersionsView>,
}

/// One point where a conversation branched: every continuation it had from there, the one
/// showing and the others in order around it. Regenerating an answer branches after its prompt;
/// editing a sent message branches at that message.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    /// Message index where the branches start.
    #[serde(alias = "prompt")]
    pub from: usize,
    /// `answer` (a regenerated reply) or `prompt` (an edited message). Empty for data saved before
    /// branching, whose branches start one message later.
    #[serde(default)]
    pub kind: String,
    /// The continuations not showing, in order, with the showing one at `current` between them.
    pub others: Vec<Vec<Message>>,
    /// Branch points inside each stored continuation, aligned with `others`.
    #[serde(default)]
    pub nested: Vec<Vec<Versions>>,
    pub current: usize,
}

impl Versions {
    /// Index of the first message the branches replace.
    fn start(&self) -> usize {
        if self.kind.is_empty() { self.from + 1 } else { self.from }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsView {
    /// Message index of the prompt the branches start at.
    pub from: usize,
    /// `answer` or `prompt`: which turn shows the switcher.
    pub kind: String,
    /// 0-based position of the showing branch.
    pub current: usize,
    pub total: usize,
}

/// Tags as stored: trimmed, lowercase, without a leading `#`, deduplicated, at most 12.
pub fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag: String = tag.trim().trim_start_matches('#').trim().to_lowercase().chars().take(32).collect();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out.truncate(12);
    out
}

fn is_prompt(message: &Message) -> bool {
    matches!(message, Message::User { content } if content.iter().any(|c| matches!(c, UserContent::Text(_))))
}

/// Removes the message the user typed at `prompt` and the reply to it (everything up to the
/// next typed message). Refused inside the summarized part, or where other versions branch off.
pub fn delete_exchange(conversation: &mut Conversation, prompt: usize) -> Result<(), String> {
    migrate(conversation);
    if !conversation.messages.get(prompt).is_some_and(is_prompt) {
        return Err("That is not a message you sent.".into());
    }
    let end = conversation.messages.iter().enumerate().skip(prompt + 1).find(|(_, m)| is_prompt(m)).map_or(conversation.messages.len(), |(i, _)| i);
    if conversation.summary.as_ref().is_some_and(|s| s.upto > prompt) {
        return Err("This part of the conversation is summarized for the model and cannot be removed.".into());
    }
    if conversation.branches.iter().any(|v| (prompt..end).contains(&v.start())) {
        return Err("This message has other versions; it can be edited instead.".into());
    }
    let n = end - prompt;
    conversation.messages.drain(prompt..end);
    shift_all(&mut conversation.times, prompt, end);
    shift_all(&mut conversation.ratings, prompt, end);
    shift_all(&mut conversation.rating_reasons, prompt, end);
    shift_all(&mut conversation.bookmarks, prompt, end);
    shift_all(&mut conversation.thought, prompt, end);
    shift_all(&mut conversation.notices, prompt, end);
    for v in conversation.branches.iter_mut().filter(|v| v.from >= end) {
        v.from -= n;
    }
    conversation.updated_at = Utc::now();
    Ok(())
}

/// Rewrites a message in place, without asking for a new reply: the text of a message the user
/// sent (its images kept), or the final text of an answer (the turn's tool steps kept). The
/// model reads the rewritten version from then on.
pub fn rewrite_message(conversation: &mut Conversation, index: usize, text: &str) -> Result<(), String> {
    migrate(conversation);
    if text.trim().is_empty() {
        return Err("The message cannot be empty.".into());
    }
    if conversation.summary.as_ref().is_some_and(|s| s.upto > index) {
        return Err("This part of the conversation is summarized for the model and cannot be changed.".into());
    }
    match conversation.messages.get(index) {
        Some(m) if is_prompt(m) => {
            if let Some(Message::User { content }) = conversation.messages.get_mut(index) {
                let mut replaced = false;
                let items: Vec<UserContent> = std::mem::take(content);
                let kept: Vec<UserContent> = items
                    .into_iter()
                    .filter_map(|c| match c {
                        UserContent::Text(_) if replaced => None,
                        UserContent::Text(_) => {
                            replaced = true;
                            Some(UserContent::text(text))
                        }
                        other => Some(other),
                    })
                    .collect();
                *content = kept;
            }
        }
        Some(_) => {
            // An answer: its last message with text, after the prompt that asked for it.
            let start = conversation.messages[..=index].iter().rposition(is_prompt).map_or(0, |i| i + 1);
            let at = (start..=index)
                .rev()
                .find(|i| matches!(&conversation.messages[*i], Message::Assistant(a) if a.content.iter().any(|c| matches!(c, AssistantContent::Text(t) if !t.text.trim().is_empty()))))
                .ok_or("This answer has no text to edit.")?;
            if let Message::Assistant(assistant) = &mut conversation.messages[at] {
                let mut replaced = false;
                let items: Vec<AssistantContent> = std::mem::take(&mut assistant.content);
                let kept: Vec<AssistantContent> = items
                    .into_iter()
                    .filter_map(|c| match c {
                        AssistantContent::Text(_) if replaced => None,
                        AssistantContent::Text(_) => {
                            replaced = true;
                            Some(AssistantContent::text(text))
                        }
                        other => Some(other),
                    })
                    .collect();
                assistant.content = kept;
            }
        }
        None => return Err("That message no longer exists.".into()),
    }
    conversation.updated_at = Utc::now();
    Ok(())
}

/// Bookmarks or un-bookmarks the turn ending at message `index`; the snippet is the turn's text.
pub fn set_bookmark(conversation: &mut Conversation, index: usize, on: bool) -> Result<(), String> {
    conversation.bookmarks.retain(|(i, _)| *i != index);
    if on {
        let turn = turns(conversation).into_iter().find(|t| t.index == index).ok_or("That message no longer exists.")?;
        let text: String = turn.parts.iter().filter_map(|p| if let Part::Text { text } = p { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join(" ");
        // Attached files and Markdown marks are noise in a list.
        let text = text.split("<attachment").next().unwrap_or_default();
        let flat: String = text.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '>' | '_')).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        let snippet = if flat.chars().count() > 140 { format!("{}…", flat.chars().take(140).collect::<String>()) } else { flat };
        conversation.bookmarks.push((index, if snippet.is_empty() { format!("{} message", turn.role) } else { snippet }));
        conversation.bookmarks.sort_by_key(|(i, _)| *i);
    }
    Ok(())
}

/// Keeps a branch of the conversation: the messages up to and including the turn ending at `upto`
/// (all when `None`), optionally without attached files and images. Notes keyed to dropped
/// messages go with them; other versions past the cut are not carried over.
pub fn branch_copy(conversation: &mut Conversation, upto: Option<usize>, without_files: bool) -> Result<(), String> {
    migrate(conversation);
    if let Some(upto) = upto {
        if upto >= conversation.messages.len() {
            return Err("That message no longer exists.".into());
        }
        // The turn's tool results follow its last assistant message; keep them with it.
        let mut end = upto + 1;
        while conversation.messages.get(end).is_some_and(|m| matches!(m, Message::User { content } if content.iter().all(|c| matches!(c, UserContent::ToolResult(_))))) {
            end += 1;
        }
        let len = conversation.messages.len();
        conversation.messages.truncate(end);
        if end < len {
            shift_all(&mut conversation.times, end, len);
            shift_all(&mut conversation.ratings, end, len);
            shift_all(&mut conversation.rating_reasons, end, len);
            shift_all(&mut conversation.bookmarks, end, len);
            shift_all(&mut conversation.thought, end, len);
            shift_all(&mut conversation.notices, end, len);
            conversation.branches.retain(|v| v.start() < end);
            conversation.versions = None;
            if conversation.summary.as_ref().is_some_and(|s| s.upto > end) {
                conversation.summary = None;
            }
        }
    }
    if without_files {
        for message in &mut conversation.messages {
            if let Message::User { content } = message {
                content.retain(|c| !matches!(c, UserContent::Image(_)));
                for c in content.iter_mut() {
                    if let UserContent::Text(t) = c {
                        t.text = strip_attachments(&t.text);
                    }
                }
                if content.is_empty() {
                    content.push(UserContent::text("(attachments removed)"));
                }
            }
        }
    }
    Ok(())
}

/// A message without its `<attachment …>…</attachment>` blocks.
fn strip_attachments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<attachment name=\"") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</attachment>") {
            Some(end) => rest = &rest[start + end + "</attachment>".len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// Drops entries keyed inside `start..end` and moves later ones back.
fn shift_all<T>(entries: &mut Vec<(usize, T)>, start: usize, end: usize) {
    entries.retain(|(i, _)| !(start..end).contains(i));
    for (i, _) in entries.iter_mut() {
        if *i >= end {
            *i -= end - start;
        }
    }
}

/// Index of the last message the user typed.
pub fn last_prompt(conversation: &Conversation) -> Option<usize> {
    conversation.messages.iter().rposition(|m| match m {
        Message::User { content } => content.iter().any(|c| matches!(c, UserContent::Text(_))),
        _ => false,
    })
}

/// Moves a branch point saved in the old single-point format into the tree.
fn migrate(conversation: &mut Conversation) {
    if let Some(mut v) = conversation.versions.take() {
        if v.kind.is_empty() {
            v.from += 1;
            v.kind = "answer".into();
        }
        conversation.branches.push(v);
    }
}

/// Takes the branch points that sit inside the messages from `start` on.
fn take_inner(conversation: &mut Conversation, start: usize) -> Vec<Versions> {
    let (inner, outer): (Vec<_>, Vec<_>) = std::mem::take(&mut conversation.branches).into_iter().partition(|v| v.start() > start);
    conversation.branches = outer;
    inner
}

/// Keeps everything from message `from` on as a branch before it is replaced; the new
/// continuation shows last. Branch points further along go with the stored branch.
pub fn keep_branch(conversation: &mut Conversation, from: usize, kind: &str) {
    migrate(conversation);
    let tail: Vec<Message> = conversation.messages.get(from..).map(<[Message]>::to_vec).unwrap_or_default();
    if tail.is_empty() {
        return;
    }
    let inner = take_inner(conversation, from);
    let mut point = match conversation.branches.iter().position(|v| v.start() == from) {
        Some(i) => conversation.branches.remove(i),
        None => Versions { from, kind: kind.to_string(), ..Default::default() },
    };
    point.nested.resize(point.others.len(), Vec::new());
    let at = point.current.min(point.others.len());
    point.others.insert(at, tail);
    point.nested.insert(at, inner);
    if point.others.len() > 20 {
        point.others.remove(0);
        point.nested.remove(0);
    }
    point.current = point.others.len();
    point.kind = kind.to_string();
    conversation.branches.push(point);
    conversation.branches.sort_by_key(Versions::start);
}

/// Shows branch `target` (0-based) at the branch point that starts at message `from`.
/// The answer text of every version at a branch point, in switcher order, to compare them
/// (Open WebUI's Compare Text).
pub fn version_texts(conversation: &Conversation, from: usize) -> Result<Vec<String>, String> {
    let mut conversation = conversation.clone();
    migrate(&mut conversation);
    let point = conversation.branches.iter().find(|v| v.start() == from).ok_or("There are no other versions here.")?;
    let start = point.start();
    let live: Vec<Message> = conversation.messages.get(start..).map(<[Message]>::to_vec).unwrap_or_default();
    let mut all: Vec<Vec<Message>> = point.others.clone();
    all.insert(point.current.min(all.len()), live);
    Ok(all
        .iter()
        .map(|messages| {
            // The reply: assistant text up to the next message the user typed.
            let skip = usize::from(messages.first().is_some_and(is_prompt));
            messages
                .iter()
                .skip(skip)
                .take_while(|m| !is_prompt(m))
                .filter_map(|m| match m {
                    Message::Assistant(a) => Some(a.content.iter().filter_map(|c| if let AssistantContent::Text(t) = c { Some(t.text.as_str()) } else { None }).collect::<Vec<_>>().join("\n")),
                    _ => None,
                })
                .filter(|t| !t.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .collect())
}

pub fn select_version(conversation: &mut Conversation, from: usize, target: usize) -> Result<(), String> {
    migrate(conversation);
    let index = conversation
        .branches
        .iter()
        .position(|v| v.start() == from)
        .ok_or("There are no other versions here.")?;
    let start = conversation.branches[index].start();
    if start > conversation.messages.len() || target > conversation.branches[index].others.len() {
        return Err("There is no such version.".into());
    }
    let mut point = conversation.branches.remove(index);
    point.nested.resize(point.others.len(), Vec::new());
    let live = conversation.messages.split_off(start);
    let live_inner = take_inner(conversation, start);
    let at = point.current.min(point.others.len());
    point.others.insert(at, live);
    point.nested.insert(at, live_inner);
    let chosen = point.others.remove(target);
    let chosen_inner = point.nested.remove(target);
    conversation.messages.extend(chosen);
    conversation.branches.extend(chosen_inner);
    conversation.times.retain(|(i, _)| *i < start);
    conversation.thought.retain(|(i, _)| *i < start);
    conversation.notices.retain(|(i, _)| *i < start);
    conversation.ratings.retain(|(i, _)| *i < start);
    if conversation.summary.as_ref().is_some_and(|s| s.upto > start) {
        conversation.summary = None;
    }
    point.current = target;
    conversation.branches.push(point);
    conversation.branches.sort_by_key(Versions::start);
    Ok(())
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
                        Some(Part::Reasoning { text: r.text.clone(), ms: conversation.thought.iter().find(|(k, _)| *k == index).map(|(_, ms)| *ms) })
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
                let notices = conversation.notices.iter().filter(|(k, _)| *k == index).map(|(_, text)| Part::Notice { text: text.clone() });
                let parts: Vec<Part> = notices.chain(parts).collect();
                match out.last_mut() {
                    Some(turn) if turn.role == "assistant" => {
                        turn.parts.extend(parts);
                        turn.index = index;
                    }
                    _ => out.push(Turn {
                        role: "assistant",
                        parts,
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

/// What the project's other conversations were about: titles and how each ended, for a new
/// conversation that may build on them.
pub fn project_digest(config_dir: &Path, project_id: &str, except: &str) -> Option<String> {
    let mut lines = Vec::new();
    for summary in list_conversations(config_dir).into_iter().filter(|c| c.project_id.as_deref() == Some(project_id) && c.id != except).take(8) {
        let Ok(c) = load_conversation(config_dir, &summary.id) else { continue };
        let last = c.messages.iter().rev().find_map(|m| match m {
            Message::Assistant(a) => a.content.iter().find_map(|x| match x {
                AssistantContent::Text(t) if !t.text.trim().is_empty() => Some(t.text.clone()),
                _ => None,
            }),
            _ => None,
        });
        let excerpt: String = last.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(400).collect();
        lines.push(format!("- \"{}\" ({}): {}", summary.title, summary.updated_at.format("%Y-%m-%d"), excerpt));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
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
        archived: conversation.archived,
        params: conversation.params.clone(),
        bookmarks: conversation.bookmarks.iter().map(|(i, _)| *i).collect(),
        turns: turns(conversation),
        versions: conversation
            .versions
            .iter()
            .map(|v| {
                let mut v = v.clone();
                if v.kind.is_empty() {
                    v.from += 1;
                    v.kind = "answer".into();
                }
                v
            })
            .chain(conversation.branches.iter().cloned())
            .filter(|v| v.start() <= conversation.messages.len() && !v.others.is_empty())
            .map(|v| VersionsView { from: v.start(), kind: v.kind.clone(), current: v.current, total: v.others.len() + 1 })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_exchange_is_deleted_with_its_reply() {
        let mut c = Conversation::new("c".into(), None, None);
        c.messages = vec![Message::user("one"), Message::assistant("1"), Message::user("two"), Message::assistant("2"), Message::user("three"), Message::assistant("3")];
        let now = Utc::now();
        c.times = vec![(0, now), (1, now), (2, now), (3, now), (4, now), (5, now)];
        c.ratings = vec![(3, 1), (5, -1)];
        delete_exchange(&mut c, 2).unwrap();
        assert_eq!(c.messages.len(), 4);
        assert_eq!(super::super::context::message_text(&c.messages[2]), "three");
        assert_eq!(c.ratings, vec![(3, -1)], "the deleted reply's rating goes, later ones move back");
        assert_eq!(c.times.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(delete_exchange(&mut c, 1).is_err(), "only typed messages start an exchange");
        // The last exchange can go too.
        delete_exchange(&mut c, 2).unwrap();
        assert_eq!(c.messages.len(), 2);
    }

    #[test]
    fn messages_are_rewritten_without_a_new_reply() {
        use rig_core::completion::message::AssistantContent;
        let mut c = Conversation::new("c".into(), None, None);
        let tool_turn = Message::Assistant(rig_core::completion::message::AssistantMessage::new(vec![AssistantContent::tool_call("t1", rig_core::completion::message::ToolName::new("web_search").unwrap(), serde_json::json!({ "query": "x" }))]));
        c.messages = vec![Message::user("what is 2+2?"), tool_turn, Message::assistant("five"), Message::user("thanks")];
        rewrite_message(&mut c, 0, "what is 2+3?").unwrap();
        assert_eq!(super::super::context::message_text(&c.messages[0]), "what is 2+3?");
        // Editing an answer changes its final text and keeps the tool step before it.
        rewrite_message(&mut c, 2, "Four.").unwrap();
        assert_eq!(super::super::context::message_text(&c.messages[2]), "Four.");
        assert!(matches!(&c.messages[1], Message::Assistant(a) if matches!(a.content[0], AssistantContent::ToolCall(_))));
        assert_eq!(c.messages.len(), 4);
        assert!(rewrite_message(&mut c, 3, "  ").is_err());
        assert!(rewrite_message(&mut c, 9, "x").is_err());
    }

    #[test]
    fn bookmarks_keep_a_snippet_and_follow_deletions() {
        let mut c = Conversation::new("c".into(), None, None);
        c.messages = vec![Message::user("one"), Message::assistant("**First** answer"), Message::user("two"), Message::assistant("Second answer")];
        set_bookmark(&mut c, 3, true).unwrap();
        set_bookmark(&mut c, 1, true).unwrap();
        assert_eq!(c.bookmarks, vec![(1, "First answer".to_string()), (3, "Second answer".to_string())]);
        assert!(set_bookmark(&mut c, 7, true).is_err());
        delete_exchange(&mut c, 0).unwrap();
        assert_eq!(c.bookmarks, vec![(1, "Second answer".to_string())], "the deleted answer's bookmark goes, the later one moves back");
        set_bookmark(&mut c, 1, false).unwrap();
        assert!(c.bookmarks.is_empty());
    }

    #[test]
    fn branches_keep_what_was_asked_for() {
        let mut c = Conversation::new("c".into(), None, None);
        c.messages = vec![
            Message::user("Summarise this\n\n<attachment name=\"notes.txt\">\nsecret plans\n</attachment>"),
            Message::assistant("A summary."),
            Message::user("two"),
            Message::assistant("2"),
        ];
        c.ratings = vec![(1, 1), (3, -1)];
        c.bookmarks = vec![(3, "2".into())];
        let mut upto = c.clone();
        branch_copy(&mut upto, Some(1), false).unwrap();
        assert_eq!(upto.messages.len(), 2);
        assert_eq!((upto.ratings.clone(), upto.bookmarks.len()), (vec![(1, 1)], 0), "notes on later messages go with them");
        assert!(super::super::context::message_text(&upto.messages[0]).contains("secret plans"));
        let mut bare = c.clone();
        branch_copy(&mut bare, None, true).unwrap();
        assert_eq!(bare.messages.len(), 4);
        assert_eq!(super::super::context::message_text(&bare.messages[0]), "Summarise this");
        assert!(branch_copy(&mut c.clone(), Some(9), false).is_err());
    }

    #[test]
    fn chat_settings_fall_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(chat_defaults(dir.path()), ChatParams::default());
        let saved = save_chat_defaults(dir.path(), ChatParams { temperature: Some(5.0), top_p: Some(0.9), instructions: "x".into(), ..Default::default() }).unwrap();
        assert_eq!((saved.temperature, saved.top_p, saved.instructions.as_str()), (Some(2.0), Some(0.9), ""), "clamped, and no instructions in defaults");
        let own = ChatParams { temperature: Some(0.2), ..Default::default() };
        let effective = own.or_defaults(&chat_defaults(dir.path()));
        assert_eq!((effective.temperature, effective.top_p), (Some(0.2), Some(0.9)), "a chat's own value wins; unset ones come from the defaults");
    }

    #[test]
    fn tags_are_cleaned() {
        let tags = clean_tags(&["#Work".into(), " work ".into(), "".into(), "Q4 plan".into()]);
        assert_eq!(tags, ["work", "q4 plan"]);
    }

    #[test]
    fn regenerated_answers_are_kept_as_versions() {
        let mut c = Conversation::new("v".into(), None, None);
        c.messages.push(Message::user("Q"));
        c.messages.push(Message::assistant("A1"));
        let text = |c: &Conversation| match c.messages.last() {
            Some(Message::Assistant(a)) => match a.content.first() {
                Some(rig_core::completion::message::AssistantContent::Text(t)) => t.text.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        // Regenerate twice: keep the answer, then a new one is written.
        keep_branch(&mut c, 0, "answer");
        c.messages.truncate(1);
        c.messages.push(Message::assistant("A2"));
        keep_branch(&mut c, 0, "answer");
        c.messages.truncate(1);
        c.messages.push(Message::assistant("A3"));
        let v = view(&c).versions[0].clone();
        assert_eq!((v.current, v.total), (2, 3));
        select_version(&mut c, 0, 0).unwrap();
        assert_eq!(text(&c), "A1");
        select_version(&mut c, 0, 1).unwrap();
        assert_eq!(text(&c), "A2");
        assert!(select_version(&mut c, 0, 7).is_err());
        assert_eq!(text(&c), "A2", "a bad index leaves the answer as it was");
        // Every version's answer, in switcher order, to compare.
        let from = view(&c).versions[0].from;
        assert_eq!(version_texts(&c, from).unwrap(), ["A1", "A2", "A3"]);
        // Versions saved in the old single-point format compare the same way.
        let mut old = c.clone();
        old.branches.clear();
        old.messages = vec![Message::user("Q"), Message::assistant("B2")];
        old.versions = Some(Versions { from: 0, others: vec![vec![Message::assistant("B1")]], current: 1, ..Default::default() });
        let from = view(&old).versions[0].from;
        assert_eq!(version_texts(&old, from).unwrap(), ["B1", "B2"]);
        select_version(&mut c, 0, 2).unwrap();
        assert_eq!(text(&c), "A3");
        // The branch point survives later messages; they belong to the branch showing.
        c.messages.push(Message::user("Next"));
        c.messages.push(Message::assistant("N1"));
        select_version(&mut c, 0, 0).unwrap();
        assert_eq!(c.messages.len(), 2, "branch A1 had no follow-up");
        select_version(&mut c, 0, 2).unwrap();
        assert_eq!(text(&c), "N1", "the follow-up came back with its branch");

        // Editing an earlier message branches there and keeps the old continuation.
        keep_branch(&mut c, 2, "prompt");
        c.messages.truncate(2);
        c.messages.push(Message::user("Next, edited"));
        c.messages.push(Message::assistant("E1"));
        // Both branch points are live now: the answer to Q, and the edited follow-up.
        let views = view(&c).versions;
        assert_eq!(views.len(), 2);
        let v = &views[1];
        assert_eq!((v.from, v.kind.as_str(), v.current, v.total), (2, "prompt", 1, 2));
        select_version(&mut c, 2, 0).unwrap();
        assert_eq!(text(&c), "N1");
        select_version(&mut c, 2, 1).unwrap();
        assert_eq!(text(&c), "E1");

        // A tree: switching the first answer away stores the edit branch inside it, and
        // switching back brings both the follow-up and its own branch point back.
        select_version(&mut c, 0, 0).unwrap();
        assert_eq!(c.messages.len(), 2);
        assert_eq!(view(&c).versions.len(), 1, "the inner branch point left with its branch");
        select_version(&mut c, 0, 2).unwrap();
        assert_eq!(text(&c), "E1");
        let views = view(&c).versions;
        assert_eq!(views.len(), 2, "the inner branch point came back");
        select_version(&mut c, 2, 0).unwrap();
        assert_eq!(text(&c), "N1");
    }

    #[test]
    fn old_single_branch_points_still_load() {
        let mut c = Conversation::new("o".into(), None, None);
        c.messages.push(Message::user("Q"));
        c.messages.push(Message::assistant("A2"));
        // Saved before branching: the branch held answers after prompt 0.
        c.versions = Some(Versions { from: 0, kind: String::new(), others: vec![vec![Message::assistant("A1")]], nested: vec![], current: 1 });
        let v = view(&c).versions;
        assert_eq!((v[0].from, v[0].total), (1, 2));
        select_version(&mut c, 1, 0).unwrap();
        assert_eq!(c.messages.len(), 2);
        assert!(c.versions.is_none() && c.branches.len() == 1);
    }

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
