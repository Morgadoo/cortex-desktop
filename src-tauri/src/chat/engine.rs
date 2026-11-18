//! One conversation turn: stream the model's reply, run the tools it calls, and repeat until it
//! answers, with every step saved and mirrored to the page as [`ChatEvent`]s.

use chrono::Utc;
use futures::StreamExt;
use log::{info, warn};
use rig_core::completion::message::{Message, ToolCall, ToolResultContent};
use rig_core::completion::options::Effort;
use rig_core::completion::CompletionRequest;
use rig_core::operation::Completion;
use rig_core::streaming::{Item, PartKind, StreamEvent};
use rig_core::DynModel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use super::{agents, builtins, deferred, canvas, context, knowledge, libraries, media, memory, terminal, privacy, providers, sandbox, skills, subagents, tasks};
use super::store::{self, Conversation, Instructions, Project};
use super::tools::{self, ApprovalMode, ChatTool, ASK_USER};
use super::{ChatState, EVENT, LOCAL_ACCOUNT};
use crate::account::AccountId;
use crate::tools::auth::keychain_secrets;
use crate::tools::registry::{ApprovalDecision, ToolRegistry};

/// Model calls in one turn before the engine stops a tool loop that does not converge. The last
/// step withholds the tools, so a long research run still ends in an answer.
const MAX_STEPS: usize = 24;
/// Attempts per model call when the failure is transient.
const MAX_ATTEMPTS: u32 = 3;

/// What the page sees of a turn, on [`EVENT`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    #[serde(rename_all = "camelCase")]
    Started { conversation_id: String },
    /// The user's guidance, sent while the turn ran, has joined the conversation.
    #[serde(rename_all = "camelCase")]
    Steered { conversation_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Text { conversation_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Reasoning { conversation_id: String, text: String },
    /// The model began writing a call to `name`; its arguments are still arriving.
    #[serde(rename_all = "camelCase")]
    ToolPending { conversation_id: String, name: String },
    #[serde(rename_all = "camelCase")]
    ToolStart { conversation_id: String, call_id: String, name: String, arguments: Value },
    #[serde(rename_all = "camelCase")]
    ToolEnd { conversation_id: String, call_id: String, output: String, is_error: bool },
    /// The model asked the user something; answer with `chat_answer`.
    #[serde(rename_all = "camelCase")]
    Question { conversation_id: String, call_id: String, question: String, options: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Title { conversation_id: String, title: String },
    /// A sub-agent of a `delegate` call started, moved on, or finished.
    #[serde(rename_all = "camelCase")]
    Subagent { conversation_id: String, call_id: String, index: usize, title: String, status: String, activity: String },
    /// Tokens and cost so far, and how full the model's window is.
    #[serde(rename_all = "camelCase")]
    Usage { conversation_id: String, usage: context::UsageTotals, context_window: u64 },
    /// Earlier turns are being summarized to make room.
    #[serde(rename_all = "camelCase")]
    Compacting { conversation_id: String },
    #[serde(rename_all = "camelCase")]
    Compacted { conversation_id: String, upto: usize },
    #[serde(rename_all = "camelCase")]
    Done { conversation_id: String, cancelled: bool },
    /// Something the user should know, such as a switch to the backup model.
    #[serde(rename_all = "camelCase")]
    Notice { conversation_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    Error { conversation_id: String, message: String },
}

/// An image the user attached, as the page read it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
    pub media_type: String,
    /// Base64, without a `data:` prefix.
    pub data: String,
}

/// Images per message, and the size of each once decoded.
const MAX_IMAGES: usize = 6;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// The user's message with its images, checked for type and size.
pub fn user_message(text: &str, images: &[ImageInput]) -> Result<Message, String> {
    use rig_core::completion::message::{ImageMediaType, MimeType, UserContent};
    if images.is_empty() {
        return Ok(Message::user(text));
    }
    if images.len() > MAX_IMAGES {
        return Err(format!("Attach at most {MAX_IMAGES} images."));
    }
    let mut content = vec![UserContent::text(if text.is_empty() { "What is in this image?" } else { text })];
    for image in images {
        let media = ImageMediaType::from_mime_type(&image.media_type)
            .ok_or_else(|| format!("{} images are not supported.", image.media_type))?;
        if image.data.len() / 4 * 3 > MAX_IMAGE_BYTES {
            return Err("Each image can be at most 8 MB.".into());
        }
        content.push(UserContent::image_base64(image.data.clone(), Some(media), None));
    }
    Ok(Message::User { content })
}

/// The composer's speed setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    #[default]
    Fast,
    Think,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRequest {
    pub conversation_id: String,
    /// The user's message; empty to regenerate the last reply.
    #[serde(default)]
    pub text: String,
    pub model: String,
    #[serde(default)]
    pub speed: Speed,
    #[serde(default)]
    pub approval: ApprovalMode,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Images sent with the message, for models that can see.
    #[serde(default)]
    pub images: Vec<ImageInput>,
    /// Replace the prompt at this message index with `text` and answer again from there.
    #[serde(default)]
    pub edit_index: Option<usize>,
    /// Keep the conversation out of the history; it is deleted on the next start.
    #[serde(default)]
    pub temporary: bool,
    /// The agent to run as; only read when the conversation is created.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Pick up where a failed or stopped turn left off, with nothing added or removed.
    #[serde(default)]
    pub resume: bool,
    /// Code sessions: the folder to work in. Only read when the conversation is created.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Connector ids to offer; all enabled ones when absent.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
}

pub(super) fn emit(app: &AppHandle, event: ChatEvent) {
    if let Err(e) = app.emit(EVENT, event) {
        warn!("could not emit chat event: {e}");
    }
}

const TONES: &[(&str, &str)] = &[
    ("formal", "Write in a polite, professional register."),
    ("casual", "Write casually, like a friendly colleague."),
    ("empathetic", "Be warm and empathetic; acknowledge feelings before advising."),
    ("objective", "Be objective and neutral; stick to facts and trade-offs."),
    ("concise", "Be brief: lead with the answer and skip filler."),
    ("witty", "Keep a light, good-humoured tone where it fits."),
    ("motivational", "Be encouraging and energising."),
    ("reflective", "Be thoughtful; surface assumptions and second-order effects."),
];

/// The system prompt: who the assistant is, today's date, and the user's standing instructions.
/// What the system prompt is built from, beside the tools on offer.
#[derive(Default)]
pub struct Context<'a> {
    pub instructions: Option<&'a Instructions>,
    pub project: Option<&'a Project>,
    pub workdir: Option<&'a str>,
    pub memories: Option<&'a memory::Memories>,
    pub skills: &'a [skills::Skill],
    pub canvases: &'a [canvas::Canvas],
    pub agent: Option<&'a agents::Agent>,
    pub libraries: &'a [libraries::Library],
    /// Summaries of the project's other conversations, when the project shares them.
    pub project_digest: Option<String>,
}

pub fn preamble(context: &Context<'_>, tools: &[ChatTool]) -> String {
    let default_instructions = Instructions::default();
    let instructions = context.instructions.unwrap_or(&default_instructions);
    let project = context.project;
    let workdir = context.workdir;
    let mut out = String::new();
    if let Some(agent) = context.agent {
        out.push_str(&format!("You are {}", agent.name));
        if !agent.description.trim().is_empty() {
            out.push_str(&format!(": {}", agent.description.trim()));
        }
        out.push_str("\n\n");
        if !agent.instructions.trim().is_empty() {
            out.push_str(agent.instructions.trim());
            out.push_str("\n\n");
        }
    }
    out.push_str(
        "You are a helpful, accurate assistant running in a desktop app. Answer in the user's \
         language. Use Markdown: short paragraphs, lists and tables where they help, fenced code \
         blocks with a language tag. When you are unsure, say so instead of guessing.",
    );
    out.push_str(&format!("\n\nToday is {}.", Utc::now().format("%A, %B %-d, %Y")));
    if !instructions.name.trim().is_empty() {
        out.push_str(&format!(" The user's name is {}.", instructions.name.trim()));
    }
    let tone_id = project.map(|p| p.tone.as_str()).filter(|t| !t.is_empty()).unwrap_or(&instructions.tone);
    if let Some((_, tone)) = TONES.iter().find(|(id, _)| *id == tone_id) {
        out.push_str("\n\n");
        out.push_str(tone);
    }
    if !instructions.custom.trim().is_empty() {
        out.push_str("\n\nThe user's instructions:\n");
        out.push_str(instructions.custom.trim());
    }
    if let Some(project) = project {
        out.push_str(&format!("\n\nThis conversation belongs to the project \"{}\".", project.name));
        if !project.instructions.trim().is_empty() {
            out.push_str(" Project instructions:\n");
            out.push_str(project.instructions.trim());
        }
        if let Some(library) = project.library_id.as_ref().and_then(|id| context.libraries.iter().find(|l| &l.id == id)) {
            if !library.documents.is_empty() {
                out.push_str(&format!(
                    "\nThe project's files are in the library \"{}\" ({} documents); search it with `search_library` before answering questions they may cover.",
                    library.name,
                    library.documents.len()
                ));
            }
        }
        if let Some(digest) = &context.project_digest {
            out.push_str("\nOther conversations in this project, most recent first (build on them when relevant):\n");
            out.push_str(digest);
        }
    }
    if let Some(dir) = workdir {
        out.push_str(&super::code::prompt(dir));
    }
    if tools.iter().any(|t| t.name == builtins::WEB_SEARCH) {
        out.push_str(
            "\n\nWhen an answer depends on current or specific facts, search the web first. Cite \
             what you use inline as Markdown links whose text is the site's name, like \
             ([rust-lang.org](https://www.rust-lang.org/)), right after the claim they support.",
        );
    }
    if tools.iter().any(|t| t.connector_id.is_some()) {
        out.push_str(
            "\n\nYou can use tools on the user's computer and services. Use them when they help, \
             explain briefly what you are doing, and never claim a tool ran when it did not. \
             The user may decline a call; respect that.",
        );
    }
    if tools.iter().any(|t| t.name == builtins::DELEGATE) {
        out.push_str(
            "\n\nFor work with several steps, show a plan with `update_plan` first and keep it \
             current. When a task splits into independent parts (several topics, sources or \
             files), hand them to `delegate` so sub-agents research them in parallel, then \
             combine their reports. Call independent tools in the same step; they run at once.",
        );
    }
    if let Some(memories) = context.memories.and_then(memory::prompt) {
        out.push_str(&memories);
    }
    if let Some(skills) = skills::prompt(context.skills) {
        out.push_str(&skills);
    }
    if let Some(canvases) = canvas::prompt(context.canvases) {
        out.push_str(&canvases);
    }
    if tools.iter().any(|t| t.name == builtins::SEARCH_LIBRARY) {
        if let Some(libraries) = libraries::prompt(context.libraries) {
            out.push_str(&libraries);
        }
    }
    out
}

/// The API key for `vendor` from the keychain, if one is saved.
pub(crate) async fn api_key(vendor: &str) -> Result<Option<String>, String> {
    let account = AccountId::parse(LOCAL_ACCOUNT)?;
    keychain_secrets(account)
        .load(&providers::secret_name(vendor))
        .await
}

pub async fn connect_model(config_dir: &std::path::Path, reference: &str) -> Result<DynModel<Completion>, String> {
    let (vendor, _) = providers::split_reference(reference)
        .ok_or_else(|| format!("`{reference}` is not a model reference"))?;
    let file = providers::load(config_dir);
    let settings = file.providers.get(vendor).cloned().unwrap_or_default();
    let key = if settings.has_key { api_key(vendor).await? } else { None };
    providers::connect(reference, &settings, key)
}

fn config_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::settings::get_config_dir(app)
}

/// Drops everything after the last message the user typed, so the reply is written again.
fn rewind_to_last_prompt(conversation: &mut Conversation) -> bool {
    match store::last_prompt(conversation) {
        Some(index) => {
            // The answer being replaced stays available as an earlier version.
            store::keep_branch(conversation, index, "answer");
            conversation.messages.truncate(index + 1);
            conversation.times.retain(|(i, _)| *i <= index);
            conversation.ratings.retain(|(i, _)| *i <= index);
            true
        }
        None => false,
    }
}

/// Runs a turn to its end. Always emits `Done` or `Error` last.
pub async fn run_turn(app: AppHandle, request: TurnRequest, cancel: watch::Receiver<bool>) {
    let id = request.conversation_id.clone();
    emit(&app, ChatEvent::Started { conversation_id: id.clone() });
    let outcome = turn(&app, &request, cancel).await;
    app.state::<ChatState>().finish(&id);
    match outcome {
        Ok(cancelled) => emit(&app, ChatEvent::Done { conversation_id: id, cancelled }),
        Err(message) => {
            warn!("chat turn failed: {message}");
            // A conversation whose first reply failed still gets a name in the sidebar.
            if let Ok(dir) = config_dir(&app) {
                if let Ok(mut c) = store::load_conversation(&dir, &id) {
                    if c.title.is_empty() && !c.messages.is_empty() {
                        c.title = fallback_title(&c);
                        let title = c.title.clone();
                        if store::save_conversation(&dir, &c).is_ok() {
                            emit(&app, ChatEvent::Title { conversation_id: id.clone(), title });
                        }
                    }
                }
            }
            emit(&app, ChatEvent::Error { conversation_id: id, message });
        }
    }
}

/// Waits for the turn to be cancelled; never resolves otherwise.
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Returns whether the turn was cancelled.
async fn turn(app: &AppHandle, request: &TurnRequest, mut cancel: watch::Receiver<bool>) -> Result<bool, String> {
    let config_dir = config_dir(app)?;
    let id = request.conversation_id.as_str();
    let mut conversation = match store::load_conversation(&config_dir, id) {
        Ok(c) => c,
        Err(_) => {
            let mut c = Conversation::new(id.to_string(), request.project_id.clone(), None);
            c.workdir = request.workdir.clone().filter(|d| !d.trim().is_empty());
            c.agent_id = request.agent_id.clone();
            c.temporary = request.temporary;
            c
        }
    };
    conversation.model = Some(request.model.clone());

    if let Some(index) = request.edit_index {
        let is_prompt = conversation.messages.get(index).is_some_and(|m| {
            matches!(m, Message::User { content } if content.iter().any(|c| matches!(c, rig_core::completion::message::UserContent::Text(_))))
        });
        if !is_prompt || request.text.trim().is_empty() {
            return Err("That message cannot be edited.".into());
        }
        // The old message and everything after it stay available as a branch.
        store::keep_branch(&mut conversation, index, "prompt");
        conversation.messages.truncate(index);
        conversation.times.retain(|(i, _)| *i < index);
        conversation.ratings.retain(|(i, _)| *i < index);
        if conversation.summary.as_ref().is_some_and(|s| s.upto > index) {
            conversation.summary = None;
        }
        conversation.messages.push(Message::user(request.text.trim()));
        conversation.times.push((conversation.messages.len() - 1, Utc::now()));
    } else if request.resume {
        if conversation.messages.is_empty() {
            return Err("There is nothing to continue.".into());
        }
        // A reply that ended mid-way may end on an assistant message; nudge it to go on.
        if matches!(conversation.messages.last(), Some(Message::Assistant(_))) {
            conversation.messages.push(Message::user("Continue."));
        }
    } else if request.text.trim().is_empty() {
        if !rewind_to_last_prompt(&mut conversation) {
            return Err("There is nothing to regenerate yet.".into());
        }
    } else {
        conversation.messages.push(user_message(request.text.trim(), &request.images)?);
        conversation.times.push((conversation.messages.len() - 1, Utc::now()));
    }
    conversation.updated_at = Utc::now();
    store::save_conversation(&config_dir, &conversation)?;

    let mut model = connect_model(&config_dir, &request.model).await?;
    let registry = app.state::<ToolRegistry>();
    let agent = conversation.agent_id.as_deref().and_then(|a| agents::find(&config_dir, a));
    if let Some(agent) = &agent {
        agents::check_budget(&config_dir, agent)?;
    }
    // The backup model, tried once when the main one fails before anything streamed.
    let mut fallback = agent.as_ref().and_then(|a| a.fallback_model.clone()).filter(|f| f != &request.model);
    let mut offered = tools::available(&registry, request.connectors.as_deref());
    offered.splice(1..1, builtins::definitions(&config_dir));
    offered.extend(super::openapi::tools(&config_dir));
    offered.extend(super::custom_tools::definitions(&config_dir));
    if super::computer::load(&config_dir).enabled {
        offered.push(super::computer::definition());
    }
    if let Some(agent) = &agent {
        offered.retain(|t| t.name == ASK_USER || agents::allows(agent, &t.name));
    }
    if request.approval.unattended() {
        // Nobody can answer a question during a scheduled run.
        offered.retain(|t| t.name != ASK_USER);
    }
    if agent.is_some() {
        // An agent keeps a notebook across its conversations and runs.
        offered.push(agents::notebook_tool());
    }
    if conversation.workdir.is_some() {
        // Coding sessions get a terminal for long-running commands.
        offered.extend(terminal::definitions());
    }
    if conversation.temporary {
        // A temporary chat leaves no trace: it does not add to or change knowledge.
        offered.retain(|t| t.name != builtins::REMEMBER && t.name != builtins::FORGET && t.name != builtins::KNOWLEDGE_WRITE);
    }
    // Past a few dozen connector tools, the rest load on demand (Vibe's "Found tools").
    let defer = deferred::should_defer(&offered);
    if defer {
        offered.push(deferred::find_tools_tool());
    }
    let project = conversation
        .project_id
        .as_ref()
        .and_then(|pid| store::load_projects(&config_dir).into_iter().find(|p| &p.id == pid));
    let instructions = store::load_instructions(&config_dir);
    let memories = memory::load(&config_dir);
    let mut skill_list = skills::list(&config_dir);
    if let Some(agent) = agent.as_ref().filter(|a| !a.skills.is_empty()) {
        skill_list.retain(|s| agent.skills.contains(&s.name));
    }
    let canvases = canvas::list_for(&config_dir, id);
    let library_list = libraries::list(&config_dir);
    let system = preamble(
        &Context {
            instructions: Some(&instructions),
            project: project.as_ref(),
            workdir: conversation.workdir.as_deref(),
            memories: Some(&memories),
            skills: &skill_list,
            canvases: &canvases,
            agent: agent.as_ref(),
            libraries: &library_list,
            project_digest: project
                .as_ref()
                .filter(|p| p.share_chats)
                .and_then(|p| store::project_digest(&config_dir, &p.id, id)),
        },
        &offered,
    );
    let mut system = system;
    if !conversation.params.instructions.is_empty() {
        system.push_str(&format!("\n\n## Instructions for this conversation\n{}", conversation.params.instructions));
    }
    if defer {
        system.push_str(&deferred::prompt(&offered, &conversation.loaded_tools));
    }
    if let Some(agent) = &agent {
        system.push_str(&agents::notebook_prompt(&agents::notebook(&config_dir, &agent.id)));
    }
    if offered.iter().any(|t| t.name == builtins::DELEGATE) {
        if let Some(team) = agents::team_prompt(&agents::load(&config_dir), agent.as_ref().map(|a| a.id.as_str())) {
            system.push_str(&team);
        }
    }
    // Knowledge pages that matter for this message come in full; the rest by title.
    let system = if memories.enabled {
        let query = store::last_prompt(&conversation)
            .and_then(|i| conversation.messages.get(i))
            .map(super::context::message_text)
            .unwrap_or_default();
        match knowledge::prompt(&knowledge::load(&config_dir), &query) {
            Some(section) => format!("{system}{section}"),
            None => system,
        }
    } else {
        system
    };
    let system = if request.approval == ApprovalMode::Plan {
        format!(
            "{system}\n\nPlan mode is on. Investigate with read-only tools, then present a numbered plan \
             of the changes you would make (files, commands, risks) and stop. Do not try to change \
             anything; the user approves the plan and then switches to an editing mode."
        )
    } else {
        system
    };
    let reasoning = providers::supports_reasoning(&request.model);
    let window = context::context_window(&request.model);
    // Personal data in what the user typed is swapped for placeholders before a cloud request.
    let filter = privacy::load(&config_dir).enabled
        && providers::split_reference(&request.model).is_some_and(|(vendor, _)| privacy::is_cloud(vendor));
    let mut redactions = privacy::Redactions::default();
    compact_if_needed(app, &model, &mut conversation, window, &config_dir).await;

    for step in 0..MAX_STEPS {
        // Guidance the user sent while this turn was running joins the conversation here.
        for text in app.state::<ChatState>().take_steering(id) {
            conversation.messages.push(Message::user(format!("(Sent while you were working) {text}")));
            conversation.times.push((conversation.messages.len() - 1, Utc::now()));
            emit(app, ChatEvent::Steered { conversation_id: id.into(), text });
        }
        let mut carried = context::request_messages(&conversation.messages, conversation.summary.as_ref());
        if filter {
            carried = privacy::redact_messages(&carried, &mut redactions);
        }
        let Some((prompt, history)) = carried.split_last() else {
            return Err("empty conversation".into());
        };
        let last_step = step + 1 == MAX_STEPS;
        let mut step_system = match &conversation.summary {
            Some(summary) => format!(
                "{system}\n\nSummary of the earlier part of this conversation (those messages are no longer shown):\n{}",
                summary.text
            ),
            None => system.clone(),
        };
        if filter {
            step_system = redactions.redact(&step_system);
        }
        if last_step {
            step_system.push_str("\n\nYou have used every tool step available for this request. Answer now from what you have found, and say what you could not finish.");
        }
        let mut call = CompletionRequest::new(prompt.clone())
            .messages(history.iter().cloned())
            .preamble(step_system);
        if !last_step {
            // Deferred tools go out once they are loaded; the rest every time.
            let definitions = offered
                .iter()
                .filter(|t| !defer || !deferred::deferrable(t) || conversation.loaded_tools.contains(&t.name))
                .filter_map(ChatTool::definition);
            call = call.tools(definitions);
        }
        // This conversation's own model settings.
        if let Some(t) = conversation.params.temperature {
            call = call.temperature(t);
        }
        if let Some(n) = conversation.params.max_tokens {
            call = call.max_tokens(n);
        }
        if reasoning && request.speed == Speed::Think {
            call = call.reasoning(Effort::High);
        }

        // Transient failures (dropped connections, rate limits, overloaded servers) are retried
        // with backoff, as long as nothing reached the user yet.
        let mut attempt = 0u32;
        // When the model started and stopped thinking, for "Thought for 4s".
        let mut thought_ms: Option<u32>;
        let response = loop {
            attempt += 1;
            let mut thinking: Option<(std::time::Instant, Option<std::time::Instant>)> = None;
            let mut stream = model.stream(call.clone()).map_err(|e| e.to_string())?;
            let mut streamed_text = String::new();
            let mut streamed_anything = false;
            let mut held = String::new();
            let was_cancelled = loop {
                tokio::select! {
                    _ = cancelled(&mut cancel) => break true,
                    item = stream.next() => match item {
                        None => break false,
                        Some(Err(_)) => break false, // finish() reports it
                        Some(Ok(Item::Unknown(_))) => {}
                        Some(Ok(Item::Event(event))) => match event {
                            StreamEvent::Text { text, .. } => {
                                streamed_anything = true;
                                if let Some((_, end @ None)) = &mut thinking {
                                    *end = Some(std::time::Instant::now());
                                }
                                let text = if filter { restore_streamed(&redactions, &mut held, &text) } else { text };
                                streamed_text.push_str(&text);
                                if !text.is_empty() {
                                    emit(app, ChatEvent::Text { conversation_id: id.into(), text });
                                }
                            }
                            StreamEvent::Reasoning { text, .. } => {
                                streamed_anything = true;
                                thinking.get_or_insert((std::time::Instant::now(), None));
                                emit(app, ChatEvent::Reasoning { conversation_id: id.into(), text });
                            }
                            StreamEvent::Start { kind: PartKind::ToolCall, name: Some(name), .. } => {
                                streamed_anything = true;
                                if let Some((_, end @ None)) = &mut thinking {
                                    *end = Some(std::time::Instant::now());
                                }
                                emit(app, ChatEvent::ToolPending { conversation_id: id.into(), name: name.to_string() });
                            }
                            _ => {}
                        },
                    },
                }
            };

            if was_cancelled {
                if !streamed_text.trim().is_empty() {
                    conversation.messages.push(Message::assistant(streamed_text));
                    conversation.times.push((conversation.messages.len() - 1, Utc::now()));
                }
                conversation.updated_at = Utc::now();
                store::save_conversation(&config_dir, &conversation)?;
                return Ok(true);
            }

            thought_ms = thinking.map(|(start, end)| end.unwrap_or_else(std::time::Instant::now).duration_since(start).as_millis().min(u32::MAX as u128) as u32);
            match stream.finish().await {
                Ok(response) => break response,
                Err(e) if e.is_retryable() && !streamed_anything && attempt < MAX_ATTEMPTS => {
                    let wait = std::time::Duration::from_secs(2u64.pow(attempt));
                    warn!("model call failed ({e}); retrying in {}s", wait.as_secs());
                    tokio::select! {
                        _ = cancelled(&mut cancel) => return Ok(true),
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
                Err(e) if !streamed_anything && fallback.is_some() => {
                    // Fail over to the agent's backup model, once, and say so.
                    let backup = fallback.take().unwrap_or_default();
                    warn!("model call failed ({e}); switching to {backup}");
                    match connect_model(&config_dir, &backup).await {
                        Ok(next) => {
                            model = next;
                            attempt = 0;
                            let label = backup.rsplit('/').next().unwrap_or(&backup).to_string();
                            let text = format!("The model failed ({}); continuing with {label}.", super::context::short_error(&e.to_string()));
                            conversation.notices.push((conversation.messages.len(), text.clone()));
                            emit(app, ChatEvent::Notice { conversation_id: id.into(), text });
                        }
                        Err(_) => {
                            store::save_conversation(&config_dir, &conversation)?;
                            return Err(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    store::save_conversation(&config_dir, &conversation)?;
                    return Err(e.to_string());
                }
            }
        };
        conversation.usage.add(&response.usage);
        emit(app, ChatEvent::Usage { conversation_id: id.into(), usage: conversation.usage.clone(), context_window: window });
        let message = response.message().map(|m| if filter { privacy::restore_message(m, &redactions) } else { m });
        let calls: Vec<ToolCall> = match &message {
            Some(Message::Assistant(a)) => a
                .content
                .iter()
                .filter_map(|c| match c {
                    rig_core::completion::message::AssistantContent::ToolCall(call) => Some(call.clone()),
                    _ => None,
                })
                .collect(),
            _ => response.tool_calls().cloned().collect(),
        };
        if let Some(message) = message {
            conversation.messages.push(message);
            if let Some(ms) = thought_ms {
                conversation.thought.push((conversation.messages.len() - 1, ms));
            }
        }
        conversation.updated_at = Utc::now();

        if calls.is_empty() {
            conversation.times.push((conversation.messages.len() - 1, Utc::now()));
            store::save_conversation(&config_dir, &conversation)?;
            // The user added guidance while the answer streamed: answer that too.
            if app.state::<ChatState>().has_steering(id) && step + 1 < MAX_STEPS {
                continue;
            }
            break;
        }
        store::save_conversation(&config_dir, &conversation)?;
        info!("chat step {step}: {} tool call(s)", calls.len());

        for call in &calls {
            emit(app, ChatEvent::ToolStart {
                conversation_id: id.into(),
                call_id: call.id.to_string(),
                name: call.function.name.to_string(),
                arguments: Value::Object(call.function.arguments.clone()),
            });
        }

        // Calls that wait on the user run one at a time; everything else runs at once.
        let waits_on_user = |call: &ToolCall| {
            let name = call.function.name.as_str();
            name == ASK_USER
                || (agent.as_ref().is_some_and(|a| agents::asks_first(a, name)) && !session_allowed(id, name))
                || offered.iter().find(|t| t.name == name).is_some_and(|tool| {
                    tool.connector_id.is_some()
                        && tools::needs_approval(tool, &Value::Object(call.function.arguments.clone()), agents::mode_for(agent.as_ref(), name, request.approval))
                })
        };
        // Decided once: an approval given during the step ("Always allow") must not move a call
        // out of the sequential queue after it was kept out of the concurrent batch.
        let sequential: Vec<bool> = calls.iter().map(|c| waits_on_user(c)).collect();
        let mut outcomes: Vec<Option<tools::Outcome>> = (0..calls.len()).map(|_| None).collect();
        let runner = Runner { app, config_dir: &config_dir, registry: &registry, offered: &offered, model: &model, request, id, agent: agent.as_ref() };
        let concurrent = futures::future::join_all(
            calls.iter().enumerate().filter(|(i, _)| !sequential[*i]).map(|(i, c)| {
                let runner = &runner;
                async move { (i, runner.run(c).await) }
            }),
        );
        let finished = tokio::select! {
            _ = cancelled(&mut cancel) => None,
            done = concurrent => Some(done),
        };
        match finished {
            Some(done) => {
                for (i, outcome) in done {
                    outcomes[i] = Some(outcome);
                }
            }
            None => {}
        }
        for (i, call) in calls.iter().enumerate() {
            if outcomes[i].is_some() || !sequential[i] || *cancel.borrow() {
                continue;
            }
            let outcome = if call.function.name.as_str() == ASK_USER && request.approval.unattended() {
                tools::Outcome { text: "Nobody is available to answer during this scheduled run; continue with your best judgement and state your assumptions.".into(), is_error: false }
            } else if call.function.name.as_str() == ASK_USER {
                ask_user(app, id, call, &mut cancel).await
            } else {
                tokio::select! {
                    _ = cancelled(&mut cancel) => cancelled_outcome(),
                    outcome = runner.run(call) => outcome,
                }
            };
            outcomes[i] = Some(outcome);
        }

        let mut results = Vec::with_capacity(calls.len());
        for (call, outcome) in calls.iter().zip(outcomes) {
            let outcome = outcome.unwrap_or_else(cancelled_outcome);
            emit(app, ChatEvent::ToolEnd {
                conversation_id: id.into(),
                call_id: call.id.to_string(),
                output: outcome.text.clone(),
                is_error: outcome.is_error,
            });
            if call.function.name.as_str() == deferred::FIND_TOOLS && !outcome.is_error {
                // What it found is loaded for the next step and the rest of the conversation.
                if let Ok(found) = serde_json::from_str::<Value>(&outcome.text) {
                    for name in found["matches"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()) {
                        if !conversation.loaded_tools.iter().any(|t| t == name) {
                            conversation.loaded_tools.push(name.to_string());
                        }
                    }
                }
            }
            let mut content = vec![ToolResultContent::text(outcome.text)];
            // Computer use: the screenshot goes with the result; older ones leave the history.
            if call.function.name.as_str() == super::computer::TOOL {
                if let Some(data) = super::computer::take_image(id) {
                    drop_old_screenshots(&mut conversation.messages);
                    content.push(ToolResultContent::image_base64(data, Some(rig_core::completion::message::ImageMediaType::JPEG), None));
                }
            }
            let mut result = call.result(content);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        conversation.messages.push(Message::tool_results(results));
        conversation.updated_at = Utc::now();
        store::save_conversation(&config_dir, &conversation)?;

        if *cancel.borrow() {
            return Ok(true);
        }
    }

    if conversation.title.is_empty() {
        let app = app.clone();
        let reference = request.model.clone();
        tauri::async_runtime::spawn(async move { name_conversation(app, reference, conversation).await });
    }
    Ok(false)
}

/// Restores placeholders in streamed text. A placeholder can arrive split across chunks, so an
/// unclosed `[` is held back until its `]` arrives (or it is clearly not a placeholder).
fn restore_streamed(redactions: &privacy::Redactions, held: &mut String, chunk: &str) -> String {
    held.push_str(chunk);
    let cut = match held.rfind('[') {
        Some(open) if !held[open..].contains(']') && held.len() - open < 24 => open,
        _ => held.len(),
    };
    let ready: String = held.drain(..cut).collect();
    redactions.restore(&ready)
}

fn cancelled_outcome() -> tools::Outcome {
    tools::Outcome { text: "Cancelled by the user.".into(), is_error: true }
}

/// Tools the user chose "Always allow" for under an agent's ask-first policy, by conversation.
fn session_allowances() -> &'static std::sync::Mutex<std::collections::HashSet<(String, String)>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(String, String)>>> = std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

fn session_allowed(conversation: &str, tool: &str) -> bool {
    session_allowances().lock().is_ok_and(|s| s.contains(&(conversation.to_string(), tool.to_string())))
}

fn allow_for_session(conversation: &str, tool: &str) {
    if let Ok(mut s) = session_allowances().lock() {
        s.insert((conversation.to_string(), tool.to_string()));
    }
}

/// What one tool call needs to run, shared by the calls of a step.
struct Runner<'a> {
    app: &'a AppHandle,
    config_dir: &'a std::path::Path,
    registry: &'a ToolRegistry,
    offered: &'a [ChatTool],
    model: &'a DynModel<Completion>,
    request: &'a TurnRequest,
    id: &'a str,
    agent: Option<&'a agents::Agent>,
}

impl Runner<'_> {
    async fn run(&self, call: &ToolCall) -> tools::Outcome {
        let name = call.function.name.as_str();
        let arguments = Value::Object(call.function.arguments.clone());
        if let Some(raw) = &call.function.invalid_arguments {
            return tools::Outcome { text: format!("The arguments were not a JSON object: {raw}"), is_error: true };
        }
        // The agent's "ask first" policy covers every tool, built-in and reading ones included.
        let mut asked = false;
        if self.agent.is_some_and(|a| agents::asks_first(a, name)) && !session_allowed(self.id, name) {
            if self.request.approval.unattended() {
                return tools::Outcome { text: format!("{name} asks first under this agent's policy, and nobody is watching this scheduled run to approve it."), is_error: true };
            }
            let gate = ChatTool { name: name.into(), description: String::new(), parameters: Value::Null, connector_id: Some(self.agent.map_or_else(|| "agent".to_string(), |a| a.name.clone())), tool_name: name.into(), read_only: false };
            match tools::request_approval(self.app, &gate, &arguments).await {
                ApprovalDecision::Deny => {
                    return tools::Outcome { text: "The user declined this tool call. Do not retry it; ask what they would like instead.".into(), is_error: true };
                }
                // "Always allow" lasts for this conversation, as in Vibe.
                ApprovalDecision::Always => allow_for_session(self.id, name),
                ApprovalDecision::Once => {}
            }
            asked = true;
        }
        if name == builtins::DELEGATE {
            return match subagents::parse_tasks(&arguments) {
                Ok(tasks) => {
                    let reports = subagents::run_all(
                        self.app,
                        self.config_dir,
                        self.model,
                        self.offered,
                        self.id,
                        &call.id.to_string(),
                        tasks,
                    )
                    .await;
                    tools::Outcome { text: subagents::render(&reports), is_error: reports.iter().all(|r| r.status != "done") }
                }
                Err(e) => tools::Outcome { text: e, is_error: true },
            };
        }
        if name == deferred::FIND_TOOLS {
            return tools::Outcome { text: deferred::find(self.offered, &arguments).0, is_error: false };
        }
        if name == agents::NOTEBOOK_TOOL {
            let Some(agent_id) = store::load_conversation(self.config_dir, self.id).ok().and_then(|c| c.agent_id) else {
                return tools::Outcome { text: "Only agents keep a notebook.".into(), is_error: true };
            };
            let content = arguments.get("content").and_then(Value::as_str).unwrap_or_default();
            return match agents::save_notebook(self.config_dir, &agent_id, content) {
                Ok(()) => tools::Outcome { text: "Notebook saved.".into(), is_error: false },
                Err(e) => tools::Outcome { text: e, is_error: true },
            };
        }
        if name == builtins::CANVAS {
            let text = |k: &str| arguments.get(k).and_then(Value::as_str).map(str::to_string);
            let Some(content) = text("content").filter(|c| !c.trim().is_empty()) else {
                return tools::Outcome { text: "Give the canvas content.".into(), is_error: true };
            };
            let id = text("id").filter(|i| !i.trim().is_empty());
            let kind = text("kind");
            return match canvas::write_kind(self.config_dir, self.id, id.as_deref(), text("title").as_deref(), &content, "assistant", kind.as_deref()) {
                Ok(c) => tools::Outcome {
                    text: serde_json::json!({ "id": c.id, "title": c.title, "kind": c.kind, "version": c.versions.len() }).to_string(),
                    is_error: false,
                },
                Err(e) => tools::Outcome { text: e, is_error: true },
            };
        }
        if name == sandbox::RUN_CODE {
            return self.run_code(&arguments).await;
        }
        if name == terminal::START || name == terminal::READ || name == terminal::STOP {
            return self.terminal(name, &arguments).await;
        }
        if name == media::GENERATE_IMAGE {
            return media::generate(self.config_dir, self.id, &arguments).await;
        }
        if name == builtins::NOTIFY {
            let text = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(240).collect::<String>();
            let title = text("title");
            if title.trim().is_empty() {
                return tools::Outcome { text: "Give the notification a title.".into(), is_error: true };
            }
            notify(self.app, &title, &text("body"));
            return tools::Outcome { text: "Notification shown.".into(), is_error: false };
        }
        if builtins::is_builtin(name) {
            return builtins::run(self.config_dir, name, &arguments).await;
        }
        match self.offered.iter().find(|t| t.name == name) {
            // Approved just now under the agent's policy: do not ask a second time.
            Some(tool) if asked => tools::execute(self.app, self.registry, tool, arguments, ApprovalMode::Auto, self.id).await,
            Some(tool) => {
                let mode = agents::mode_for(self.agent, name, self.request.approval);
                // Auto: what Ask mode would confirm gets a quick check against the request.
                if mode == ApprovalMode::Auto && tools::risk(tool, &arguments).is_none() && tools::needs_approval(tool, &arguments, ApprovalMode::Ask) {
                    if let Some(reason) = self.auto_check(name, &arguments).await {
                        if tools::request_approval_because(self.app, tool, &arguments, Some(&reason)).await == ApprovalDecision::Deny {
                            return tools::Outcome { text: "The user declined this tool call. Do not retry it; ask what they would like instead.".into(), is_error: true };
                        }
                    }
                }
                tools::execute(self.app, self.registry, tool, arguments, mode, self.id).await
            }
            None => tools::Outcome { text: format!("There is no tool named {name}."), is_error: true },
        }
    }
}

impl Runner<'_> {
    /// Background commands in the session's folder; starting one asks like any shell command.
    async fn terminal(&self, name: &str, arguments: &Value) -> tools::Outcome {
        let fail = |text: &str| tools::Outcome { text: text.into(), is_error: true };
        let arg = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        match name {
            terminal::READ => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => {
                    let chars = arguments.get("chars").and_then(Value::as_u64).unwrap_or(4000).clamp(200, 20_000) as usize;
                    tools::Outcome {
                        text: format!("Status: {}{}\n\n{}", view.status, view.exit_code.map(|c| format!(" (exit {c})")).unwrap_or_default(), terminal::tail(&view, chars)),
                        is_error: false,
                    }
                }
                _ => fail("There is no background command with that id here."),
            },
            terminal::STOP => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => match terminal::stop(&view.id) {
                    Ok(()) => tools::Outcome { text: "Stopped.".into(), is_error: false },
                    Err(e) => fail(&e),
                },
                _ => fail("There is no background command with that id here."),
            },
            _ => {
                let Some(dir) = store::load_conversation(self.config_dir, self.id).ok().and_then(|c| c.workdir) else {
                    return fail("Background commands need a coding session with a folder.");
                };
                let gate = ChatTool {
                    name: "terminal__start".into(),
                    description: String::new(),
                    parameters: Value::Null,
                    connector_id: Some("terminal".into()),
                    tool_name: "start".into(),
                    read_only: false,
                };
                // Smart-approve: a command that is hard to undo asks even in Full auto.
                let risky = tools::risk(&gate, arguments);
                match agents::mode_for(self.agent, terminal::START, self.request.approval) {
                    ApprovalMode::Plan => return fail("Plan mode: nothing is started yet. Put this command in your plan."),
                    ApprovalMode::Unattended => return fail("This scheduled run may not start commands."),
                    ApprovalMode::Autonomous if risky.is_some() => {
                        return fail(&format!("This command was not started: {}, and nobody is watching this scheduled run to approve it.", risky.unwrap_or_default()))
                    }
                    ApprovalMode::Autonomous if risky.is_none() => {}
                    // Auto: the harness evaluates the command against the request.
                    ApprovalMode::Auto if risky.is_none() => {
                        if let Some(reason) = self.auto_check(terminal::START, arguments).await {
                            if tools::request_approval_because(self.app, &gate, arguments, Some(&reason)).await == ApprovalDecision::Deny {
                                return fail("The user declined to start this command. Do not retry it; ask what they would like instead.");
                            }
                        }
                    }
                    _ => {
                        if tools::request_approval(self.app, &gate, arguments).await == ApprovalDecision::Deny {
                            return fail("The user declined to start this command. Do not retry it; ask what they would like instead.");
                        }
                    }
                }
                match terminal::start(self.id, &dir, &arg("command")) {
                    Ok(id) => {
                        // A moment for the first lines (a server's address, an early error).
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        let view = terminal::get(&id);
                        tools::Outcome {
                            text: format!(
                                "Started in the background with id {id}. Status: {}.\n{}",
                                view.as_ref().map_or("running", |v| v.status.as_str()),
                                view.as_ref().map(|v| terminal::tail(v, 2000)).unwrap_or_default()
                            ),
                            is_error: false,
                        }
                    }
                    Err(e) => fail(&e),
                }
            }
        }
    }

    /// Auto mode's judgement on one change: `None` to go ahead, or why to ask the user.
    async fn auto_check(&self, tool: &str, arguments: &Value) -> Option<String> {
        let conversation = store::load_conversation(self.config_dir, self.id).ok()?;
        let request = store::last_prompt(&conversation)
            .and_then(|i| conversation.messages.get(i))
            .map(super::context::message_text)
            .unwrap_or_default();
        let prompt = tools::auto_check_prompt(&request, tool, arguments);
        let call = rig_core::completion::CompletionRequest::new(Message::user(prompt));
        match self.model.call(call).await {
            Ok(response) => tools::auto_check_verdict(&response.text()),
            // No answer: asking is the safe side.
            Err(e) => Some(format!("the safety check could not run ({})", super::context::short_error(&e.to_string()))),
        }
    }

    /// Runs code once the user allows it: every run in "ask" modes, unless they allowed all.
    async fn run_code(&self, arguments: &Value) -> tools::Outcome {
        match self.request.approval {
            ApprovalMode::Unattended => {
                return tools::Outcome {
                    text: "This scheduled run may not run code; the user can allow changes for this task.".into(),
                    is_error: true,
                }
            }
            ApprovalMode::Plan => {
                return tools::Outcome {
                    text: "Plan mode: code is not run yet. Put this step in your plan instead.".into(),
                    is_error: true,
                }
            }
            ApprovalMode::Autonomous | ApprovalMode::Auto => {}
            ApprovalMode::Ask | ApprovalMode::AcceptEdits if sandbox::always_allowed(self.config_dir) => {}
            ApprovalMode::Ask | ApprovalMode::AcceptEdits => {
                let Some(tool) = self.offered.iter().find(|t| t.name == sandbox::RUN_CODE) else {
                    return tools::Outcome { text: "Running code is not available.".into(), is_error: true };
                };
                let gate = ChatTool { connector_id: Some("sandbox".into()), ..tool.clone() };
                match tools::request_approval(self.app, &gate, arguments).await {
                    ApprovalDecision::Deny => {
                        return tools::Outcome {
                            text: "The user declined to run this code. Do not retry it; ask what they would like instead.".into(),
                            is_error: true,
                        }
                    }
                    ApprovalDecision::Always => {
                        if let Err(e) = sandbox::set_always_allowed(self.config_dir, true) {
                            warn!("could not save the code-run permission: {e}");
                        }
                    }
                    ApprovalDecision::Once => {}
                }
            }
        }
        sandbox::run(self.config_dir, self.id, arguments).await
    }
}

/// Puts the model's question to the user and waits for the answer.
async fn ask_user(app: &AppHandle, id: &str, call: &ToolCall, cancel: &mut watch::Receiver<bool>) -> tools::Outcome {
    let arguments = Value::Object(call.function.arguments.clone());
    let call_id = call.id.to_string();
    let question = arguments.get("question").and_then(Value::as_str).unwrap_or_default().to_string();
    let options = arguments
        .get("options")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    let state = app.state::<ChatState>();
    let answer = state.ask(&call_id);
    emit(app, ChatEvent::Question { conversation_id: id.into(), call_id: call_id.clone(), question, options });
    tokio::select! {
        _ = cancelled(cancel) => {
            state.forget_question(&call_id);
            tools::Outcome { text: "The user dismissed the question.".into(), is_error: true }
        }
        answer = answer => match answer {
            Ok(text) => tools::Outcome { text, is_error: false },
            Err(_) => tools::Outcome { text: "The user dismissed the question.".into(), is_error: true },
        }
    }
}

/// Summarizes the turns before the last few once the conversation nears the model's window.
async fn compact_if_needed(
    app: &AppHandle,
    model: &DynModel<Completion>,
    conversation: &mut Conversation,
    window: u64,
    config_dir: &std::path::Path,
) {
    let measured = if conversation.usage.context_tokens > 0 {
        conversation.usage.context_tokens
    } else {
        context::estimate_tokens(&context::request_messages(&conversation.messages, conversation.summary.as_ref()))
    };
    if !context::should_compact(measured, window) {
        return;
    }
    let covered = conversation.summary.as_ref().map_or(0, |s| s.upto);
    let Some(point) = context::compaction_point(&conversation.messages, covered) else { return };
    let id = conversation.id.clone();
    emit(app, ChatEvent::Compacting { conversation_id: id.clone() });
    let transcript = context::transcript(&conversation.messages[covered..point]);
    let prompt = context::summary_prompt(conversation.summary.as_ref().map(|s| s.text.as_str()), &transcript);
    let request = CompletionRequest::new(Message::user(prompt)).max_tokens(1500u64);
    match tokio::time::timeout(std::time::Duration::from_secs(120), model.call(request)).await {
        Ok(Ok(response)) if !response.text().trim().is_empty() => {
            info!("compacted conversation {id} up to message {point}");
            conversation.usage.add(&response.usage);
            conversation.summary = Some(context::Summary { text: response.text().trim().to_string(), upto: point, created_at: Utc::now() });
            // The next reply measures the smaller context afresh.
            conversation.usage.context_tokens = 0;
            if let Err(e) = store::save_conversation(config_dir, conversation) {
                warn!("could not save compacted conversation: {e}");
            }
            emit(app, ChatEvent::Compacted { conversation_id: id, upto: point });
        }
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
            warn!("compaction failed; continuing with tool-result clearing only");
            emit(app, ChatEvent::Compacted { conversation_id: id, upto: covered });
        }
    }
}

/// Runs a scheduled task: a new unread conversation with the task's prompt, under its agent.
pub async fn run_task(app: AppHandle, task: tasks::Task) {
    let Ok(config_dir) = config_dir(&app) else { return };
    let agent = task.agent_id.as_deref().and_then(|a| agents::find(&config_dir, a));
    let providers_file = providers::load(&config_dir);
    let model = task
        .model
        .clone()
        .or_else(|| agent.as_ref().and_then(|a| a.model.clone()))
        .or(providers_file.default_model.clone());
    let Some(model) = model else {
        warn!("scheduled task {} has no model to run on", task.title);
        tasks::record(&config_dir, &task.id, "", "failed");
        return;
    };
    let id = uuid::Uuid::new_v4().to_string();
    let mut conversation = Conversation::new(id.clone(), None, Some(model.clone()));
    conversation.title = task.title.clone();
    conversation.agent_id = task.agent_id.clone();
    conversation.task_id = Some(task.id.clone());
    conversation.unread = true;
    if let Err(e) = store::save_conversation(&config_dir, &conversation) {
        warn!("could not start scheduled task {}: {e}", task.title);
        return;
    }
    let Some(cancel) = app.state::<ChatState>().begin(&id) else { return };
    let request = TurnRequest {
        conversation_id: id.clone(),
        text: task.prompt.clone(),
        model,
        speed: if agent.as_ref().is_some_and(|a| a.think) { Speed::Think } else { Speed::Fast },
        approval: if task.allow_changes { ApprovalMode::Autonomous } else { ApprovalMode::Unattended },
        project_id: None,
        agent_id: task.agent_id.clone(),
        images: Vec::new(),
        edit_index: None,
        temporary: false,
        resume: false,
        workdir: None,
        connectors: None,
    };
    run_turn(app.clone(), request, cancel).await;
    let finished = store::load_conversation(&config_dir, &id).ok();
    let answered = finished
        .as_ref()
        .and_then(|c| c.messages.last())
        .is_some_and(|m| matches!(m, Message::Assistant(_)));
    tasks::record(&config_dir, &task.id, &id, if answered { "ok" } else { "failed" });
    notify(&app, &task.title, if answered { "Your scheduled task finished." } else { "Your scheduled task could not finish." });
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("could not show a notification: {e}");
    }
}

/// A message without the composer's directives ("Use a canvas…", "Use the … skill.", quoted
/// selections), which say how to answer rather than what the conversation is about.
pub fn strip_directives(text: &str) -> String {
    let mut rest = text.trim_start();
    loop {
        if let Some(r) = rest.strip_prefix("Use a canvas for this.") {
            rest = r.trim_start();
        } else if let Some(r) = rest.strip_prefix("Use the \"").and_then(|r| r.split_once("\" skill.")).map(|(_, r)| r) {
            rest = r.trim_start();
        } else if rest.starts_with("> ") {
            rest = rest.split_once("\n\n").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    rest.to_string()
}

/// The first user message, cut to a sidebar-sized title.
pub fn fallback_title(conversation: &Conversation) -> String {
    let first = conversation
        .messages
        .iter()
        .find_map(|m| match m {
            Message::User { content } => content.iter().find_map(|c| match c {
                rig_core::completion::message::UserContent::Text(t) => Some(t.text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();
    let line = strip_directives(&first).lines().next().unwrap_or_default().trim().to_string();
    if line.chars().count() > 48 {
        format!("{}…", line.chars().take(47).collect::<String>().trim_end())
    } else if line.is_empty() {
        "New chat".into()
    } else {
        line
    }
}

/// Asks the same model for a short title, falling back to the first message.
async fn name_conversation(app: AppHandle, reference: String, mut conversation: Conversation) {
    let Ok(config_dir) = config_dir(&app) else { return };
    let mut title = fallback_title(&conversation);
    if let Ok(model) = connect_model(&config_dir, &reference).await {
        let prompt = format!(
            "Write a title of at most 6 words for a conversation that starts with the message \
             below, in the message's language. Reply with the title only, no quotes.\n\n{}",
            title
        );
        let request = CompletionRequest::new(Message::user(prompt)).max_tokens(32u64);
        if let Ok(Ok(response)) =
            tokio::time::timeout(std::time::Duration::from_secs(20), model.call(request)).await
        {
            let text = response.text();
            let text = text.trim().trim_matches(['"', '\'', '*', '#']).trim();
            if !text.is_empty() && text.chars().count() <= 80 {
                title = text.to_string();
            }
        }
    }
    // The conversation may have moved on while the title was written; only the title changes.
    let mut latest = store::load_conversation(&config_dir, &conversation.id).unwrap_or_else(|_| {
        conversation.title.clone_from(&title);
        conversation.clone()
    });
    if latest.title.is_empty() {
        latest.title.clone_from(&title);
        if store::save_conversation(&config_dir, &latest).is_ok() {
            emit(&app, ChatEvent::Title { conversation_id: latest.id, title });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_carries_instructions_and_project() {
        let instructions = Instructions {
            tone: "concise".into(),
            custom: "Use metric units.".into(),
            name: "Ana".into(),
        };
        let project = Project {
            id: "p".into(),
            name: "Garden".into(),
            instructions: "Plants only.".into(),
            tone: String::new(),
            icon: String::new(),
            library_id: None,
            share_chats: false,
        };
        let text = preamble(
            &Context { instructions: Some(&instructions), project: Some(&project), workdir: Some("/src/app"), ..Default::default() },
            &[],
        );
        assert!(text.contains("Ana"));
        assert!(text.contains("Be brief"));
        assert!(text.contains("Use metric units."));
        assert!(text.contains("\"Garden\""));
        assert!(text.contains("Plants only."));
        assert!(text.contains("`/src/app`"));
        assert!(!text.contains("tools on the user's computer"));
    }

    #[test]
    fn regenerating_drops_the_last_reply() {
        let mut c = Conversation::new("c".into(), None, None);
        c.messages.push(Message::user("one"));
        c.messages.push(Message::assistant("first"));
        c.messages.push(Message::user("two"));
        c.messages.push(Message::assistant("second"));
        c.ratings.push((3, 1));
        assert!(rewind_to_last_prompt(&mut c));
        assert_eq!(c.messages.len(), 3);
        assert!(c.ratings.is_empty());
    }

    #[test]
    fn images_ride_with_the_message() {
        let png = ImageInput { media_type: "image/png".into(), data: "iVBORw0KGgo=".into() };
        let m = user_message("look", &[png.clone()]).unwrap();
        let Message::User { content } = m else { panic!() };
        assert_eq!(content.len(), 2);
        assert!(user_message("x", &[ImageInput { media_type: "image/tiff".into(), data: "AA==".into() }]).is_err());
        assert!(user_message("x", &vec![png; 7]).is_err());
        assert_eq!(user_message("plain", &[]).unwrap(), Message::user("plain"));
    }

    #[test]
    fn streamed_placeholders_are_restored_across_chunks() {
        let mut r = privacy::Redactions::default();
        r.redact("mail ana@x.pt");
        let mut held = String::new();
        let mut out = String::new();
        for chunk in ["Writing to [EM", "AIL_1", "] now, [see] notes"] {
            out.push_str(&restore_streamed(&r, &mut held, chunk));
        }
        out.push_str(&r.restore(&held));
        assert_eq!(out, "Writing to ana@x.pt now, [see] notes");
    }

    #[test]
    fn titles_ignore_composer_directives() {
        assert_eq!(strip_directives("Use a canvas for this.\n\nUse the \"deep-research\" skill.\n\nAI in 2026?"), "AI in 2026?");
        assert_eq!(strip_directives("> quoted line\n> more\n\nWhat does it mean?"), "What does it mean?");
        assert_eq!(strip_directives("Plain question"), "Plain question");
    }

    #[test]
    fn titles_fall_back_to_the_first_line() {
        let mut c = Conversation::new("c".into(), None, None);
        assert_eq!(fallback_title(&c), "New chat");
        c.messages.push(Message::user("Plan a trip to Lisbon\nwith kids"));
        assert_eq!(fallback_title(&c), "Plan a trip to Lisbon");
        c.messages[0] = Message::user("x".repeat(100));
        assert_eq!(fallback_title(&c).chars().count(), 48);
    }

    #[test]
    fn events_serialize_for_the_page() {
        let event = ChatEvent::ToolEnd {
            conversation_id: "c".into(),
            call_id: "1".into(),
            output: "ok".into(),
            is_error: false,
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({"type": "toolEnd", "conversationId": "c", "callId": "1", "output": "ok", "isError": false})
        );
    }
}

/// Replaces screenshots in earlier tool results with a note: only the latest view matters, and
/// each one costs many tokens.
fn drop_old_screenshots(messages: &mut [Message]) {
    use rig_core::completion::message::UserContent;
    for message in messages.iter_mut() {
        let Message::User { content } = message else { continue };
        for item in content.iter_mut() {
            if let UserContent::ToolResult(result) = item {
                let has_image = result.content.iter().any(|c| matches!(c, ToolResultContent::Image(_)));
                if has_image {
                    let kept: Vec<ToolResultContent> = result.content.iter().filter(|c| !matches!(c, ToolResultContent::Image(_))).cloned().chain(std::iter::once(ToolResultContent::text("[earlier screenshot removed]"))).collect();
                    result.content = kept;
                }
            }
        }
    }
}
