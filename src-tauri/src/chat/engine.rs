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

use std::sync::Arc;

use cortex_kernel::agent_loop::{Reply, Steps};

use crate::chat::host::{Host, TauriHost};
use crate::chat::store::{self, Conversation, Instructions, Project};
use crate::chat::tools::{self, ApprovalMode, ChatTool, ASK_USER};
use crate::chat::{
    agents, builtins, canvas, context, deferred, knowledge, libraries, media, memory, privacy,
    providers, sandbox, skills, subagents, tasks, terminal,
};
use crate::chat::{ChatState, EVENT};
use crate::tools::registry::ApprovalDecision;

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
    Steered {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Reasoning {
        conversation_id: String,
        text: String,
    },
    /// The model began writing a call to `name`; its arguments are still arriving.
    #[serde(rename_all = "camelCase")]
    ToolPending {
        conversation_id: String,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolStart {
        conversation_id: String,
        call_id: String,
        name: String,
        arguments: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolEnd {
        conversation_id: String,
        call_id: String,
        output: String,
        is_error: bool,
    },
    /// The model asked the user something; answer with `chat_answer`.
    #[serde(rename_all = "camelCase")]
    Question {
        conversation_id: String,
        call_id: String,
        question: String,
        options: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Title {
        conversation_id: String,
        title: String,
    },
    /// A sub-agent of a `delegate` call started, moved on, or finished.
    #[serde(rename_all = "camelCase")]
    Subagent {
        conversation_id: String,
        call_id: String,
        index: usize,
        title: String,
        status: String,
        activity: String,
    },
    /// Tokens and cost so far, and how full the model's window is.
    #[serde(rename_all = "camelCase")]
    Usage {
        conversation_id: String,
        usage: context::UsageTotals,
        context_window: u64,
    },
    /// Earlier turns are being summarized to make room.
    #[serde(rename_all = "camelCase")]
    Compacting { conversation_id: String },
    #[serde(rename_all = "camelCase")]
    Compacted {
        conversation_id: String,
        upto: usize,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        conversation_id: String,
        cancelled: bool,
    },
    /// Something the user should know, such as a switch to the backup model.
    #[serde(rename_all = "camelCase")]
    Notice {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        conversation_id: String,
        message: String,
    },
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
    let mut content = vec![UserContent::text(if text.is_empty() {
        "What is in this image?"
    } else {
        text
    })];
    for image in images {
        let media = ImageMediaType::from_mime_type(&image.media_type)
            .ok_or_else(|| format!("{} images are not supported.", image.media_type))?;
        if image.data.len() / 4 * 3 > MAX_IMAGE_BYTES {
            return Err("Each image can be at most 8 MB.".into());
        }
        content.push(UserContent::image_base64(
            image.data.clone(),
            Some(media),
            None,
        ));
    }
    Ok(Message::User { content })
}

/// The composer's speed setting: how much the model reasons (Vibe's thinking levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    /// No extended reasoning.
    #[default]
    Fast,
    /// A little reasoning.
    Low,
    /// Reasoning for complex tasks.
    Think,
    /// The most the model offers short of its maximum.
    Deep,
}

impl Speed {
    /// The reasoning effort asked of the model, if any.
    pub fn effort(self) -> Option<Effort> {
        match self {
            Speed::Fast => None,
            Speed::Low => Some(Effort::Low),
            Speed::Think => Some(Effort::High),
            Speed::Deep => Some(Effort::XHigh),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
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
    /// Why Auto chose this request's model, shown above the answer.
    #[serde(skip)]
    pub route_note: Option<String>,
    /// Built-in tools turned off for this chat in the composer's Tools menu (`generate_image`,
    /// `web_search`, …).
    #[serde(default)]
    pub tools_off: Vec<String>,
    /// Model calls before the turn must answer; [`MAX_STEPS`] when absent.
    #[serde(default)]
    pub max_steps: Option<usize>,
    /// Offer only these tools, by name. For runs with a narrow job, such as a headless run that
    /// only reads.
    #[serde(default)]
    pub only_tools: Option<Vec<String>>,
    /// The whole system prompt, for a run that brings its own brief (a `delegate` worker).
    #[serde(default)]
    pub system: Option<String>,
    /// Skills left out of this turn, to measure what one contributes (an eval's ablation).
    #[serde(default)]
    pub skills_off: Vec<String>,
    /// Saved facts (by id) and knowledge pages (by id) left out of this turn, and whether the
    /// user's standing instructions are: the rest of an eval's ablation.
    #[serde(default)]
    pub facts_off: Vec<String>,
    #[serde(default)]
    pub pages_off: Vec<String>,
    #[serde(default)]
    pub instructions_off: bool,
}

pub(super) fn emit(app: &AppHandle, event: ChatEvent) {
    if let (Some(state), Ok(value)) = (app.try_state::<ChatState>(), serde_json::to_value(&event)) {
        state.record(&value);
    }
    if let Err(e) = app.emit(EVENT, event) {
        warn!("could not emit chat event: {e}");
    }
}

const TONES: &[(&str, &str)] = &[
    ("formal", "Write in a polite, professional register."),
    ("casual", "Write casually, like a friendly colleague."),
    (
        "empathetic",
        "Be warm and empathetic; acknowledge feelings before advising.",
    ),
    (
        "objective",
        "Be objective and neutral; stick to facts and trade-offs.",
    ),
    ("concise", "Be brief: lead with the answer and skip filler."),
    ("witty", "Keep a light, good-humoured tone where it fits."),
    ("motivational", "Be encouraging and energising."),
    (
        "reflective",
        "Be thoughtful; surface assumptions and second-order effects.",
    ),
];

/// The user's date and time zone, so "today" and deadlines mean what they mean to the user
/// rather than in UTC. No clock time: the preamble then stays the same all day, which keeps
/// provider prompt caches warm.
pub fn local_time_line<Tz: chrono::TimeZone>(now: chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let zone = iana_time_zone::get_timezone()
        .ok()
        .filter(|z| !z.is_empty());
    let offset = now.format("%:z").to_string();
    let offset = if offset == "+00:00" {
        "UTC".to_string()
    } else {
        format!("UTC{offset}")
    };
    format!(
        "Today is {} in the user's time zone ({}).",
        now.format("%A, %B %-d, %Y"),
        zone.map_or(offset.clone(), |z| format!("{z}, {offset}"))
    )
}

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
    /// The user's latest message, to pick what fits it when there is too much to list.
    pub query: &'a str,
}

pub fn preamble(context: &Context<'_>, tools: &[ChatTool]) -> String {
    preamble_prompt(context, tools).render()
}

/// The system prompt's base, as named sections (see [`crate::chat::prompt`]).
pub fn preamble_prompt(context: &Context<'_>, tools: &[ChatTool]) -> crate::chat::prompt::Prompt {
    use crate::chat::prompt::{Prompt, ESSENTIAL, OPTIONAL, USEFUL};
    let default_instructions = Instructions::default();
    let instructions = context.instructions.unwrap_or(&default_instructions);
    let project = context.project;
    let workdir = context.workdir;
    let mut p = Prompt::new();
    let mut identity = String::new();
    if let Some(agent) = context.agent {
        identity.push_str(&format!("You are {}", agent.label()));
        if !agent.description.trim().is_empty() {
            identity.push_str(&format!(": {}", agent.description.trim()));
        }
        identity.push_str("\n\n");
        identity.push_str(&agent.role_prompt());
        if !agent.instructions.trim().is_empty() {
            identity.push_str(agent.instructions.trim());
            identity.push_str("\n\n");
        }
        let own: Vec<&libraries::Library> = agent
            .libraries
            .iter()
            .filter_map(|id| context.libraries.iter().find(|l| &l.id == id))
            .filter(|l| !l.documents.is_empty())
            .collect();
        if !own.is_empty() {
            let names: Vec<String> = own
                .iter()
                .map(|l| format!("\"{}\" ({} documents)", l.name, l.documents.len()))
                .collect();
            identity.push_str(&format!(
                "Your reference material is in the {} {}; search it with `search_library` before answering questions it may cover, and cite what you use.\n\n",
                if own.len() == 1 { "library" } else { "libraries" },
                names.join(", ")
            ));
        }
    }
    identity.push_str(
        "You are a helpful, accurate assistant running in a desktop app. Answer in the user's \
         language. Use Markdown: short paragraphs, lists and tables where they help, fenced code \
         blocks with a language tag. When you write a text for the user to send or publish (an \
         email, a message, a post, a letter), put that text alone between a `:::writing` line and a \
         `:::` line, so they can copy it. When you are unsure, say so instead of guessing.",
    );
    identity.push_str(&format!("\n\n{}", local_time_line(chrono::Local::now())));
    if !instructions.name.trim().is_empty() {
        identity.push_str(&format!(
            " The user's name is {}.",
            instructions.name.trim()
        ));
    }
    let tone_id = project
        .map(|p| p.tone.as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or(&instructions.tone);
    if let Some((_, tone)) = TONES.iter().find(|(id, _)| *id == tone_id) {
        identity.push_str("\n\n");
        identity.push_str(tone);
    }
    p.push("identity", ESSENTIAL, identity);
    if !instructions.custom.trim().is_empty() {
        p.push(
            "your instructions",
            ESSENTIAL,
            format!(
                "\n\nThe user's instructions:\n{}",
                instructions.custom.trim()
            ),
        );
    }
    if let Some(project) = project {
        let mut text = format!(
            "\n\nThis conversation belongs to the project \"{}\".",
            project.name
        );
        if !project.instructions.trim().is_empty() {
            text.push_str(" Project instructions:\n");
            text.push_str(project.instructions.trim());
        }
        if let Some(library) = project
            .library_id
            .as_ref()
            .and_then(|id| context.libraries.iter().find(|l| &l.id == id))
        {
            if !library.documents.is_empty() {
                text.push_str(&format!(
                    "\nThe project's files are in the library \"{}\" ({} documents); search it with `search_library` before answering questions they may cover.",
                    library.name,
                    library.documents.len()
                ));
            }
        }
        p.push("project", ESSENTIAL, text);
        if let Some(digest) = &context.project_digest {
            p.push(
                "project's other chats",
                OPTIONAL,
                format!("\nOther conversations in this project, most recent first (build on them when relevant):\n{digest}"),
            );
        }
    }
    if let Some(dir) = workdir {
        p.push("code session", ESSENTIAL, crate::chat::code::prompt(dir));
    }
    let mut guidance = String::new();
    if tools.iter().any(|t| t.name == builtins::WEB_SEARCH) {
        guidance.push_str(
            "\n\nWhen an answer depends on current or specific facts, search the web first. Cite \
             what you use inline as Markdown links whose text is the site's name, like \
             ([rust-lang.org](https://www.rust-lang.org/)), right after the claim they support.",
        );
    }
    if tools.iter().any(|t| t.connector_id.is_some()) {
        guidance.push_str(
            "\n\nYou can use tools on the user's computer and services. Use them when they help, \
             explain briefly what you are doing, and never claim a tool ran when it did not. \
             The user may decline a call; respect that.",
        );
    }
    if tools.iter().any(|t| t.name == builtins::DELEGATE) {
        guidance.push_str(
            "\n\nFor work with several steps, show a plan with `update_plan` first and keep it \
             current. When a task splits into independent parts (several topics, sources or \
             files), hand them to `delegate` so sub-agents research them in parallel, then \
             combine their reports. Call independent tools in the same step; they run at once.",
        );
    }
    p.push("how to use tools", USEFUL, guidance);
    if let Some(memories) = context.memories.and_then(|m| {
        memory::prompt_for(
            m,
            context.query,
            project.map(|p| p.id.as_str()),
            context.agent.map(|a| a.id.as_str()),
        )
    }) {
        p.push("memories", USEFUL, memories);
    }
    if let Some(skills) = skills::prompt_for(context.skills, context.query) {
        p.push("skills", USEFUL, skills);
    }
    if let Some(canvases) = canvas::prompt(context.canvases) {
        p.push("canvases", OPTIONAL, canvases);
    }
    if tools.iter().any(|t| t.name == builtins::SEARCH_LIBRARY) {
        if let Some(libraries) = libraries::prompt(context.libraries) {
            p.push("libraries", USEFUL, libraries);
        }
    }
    p
}

pub async fn connect_model(
    config_dir: &std::path::Path,
    reference: &str,
) -> Result<DynModel<Completion>, String> {
    // The app's own errands on Auto use its fast model.
    let routed;
    let reference = if reference == crate::chat::router::AUTO {
        routed = crate::chat::router::models(&providers::load(config_dir))
            .map(|(fast, _)| fast)
            .ok_or("Choose Auto's fast and strong models first.")?;
        routed.as_str()
    } else {
        reference
    };
    if reference.starts_with("local/") {
        return crate::chat::local_models::connect(reference).await;
    }
    let (vendor, _) = providers::split_reference(reference)
        .ok_or_else(|| format!("`{reference}` is not a model reference"))?;
    let file = providers::load(config_dir);
    let settings = file.providers.get(vendor).cloned().unwrap_or_default();
    let key = if settings.has_key {
        crate::chat::secrets::api_key(vendor).await?
    } else {
        None
    };
    providers::connect(reference, &settings, key)
}

/// One request for a text, for the app's own errands (titles, follow-ups, translations,
/// rewrites): when the model fails or does not answer in time, the backup model set under
/// Providers is tried once, as chats do. The error is the first model's, in plain words.
pub async fn ask_text(
    config_dir: &std::path::Path,
    reference: &str,
    request: CompletionRequest,
    secs: u64,
) -> Result<String, String> {
    let attempt = |reference: String, request: CompletionRequest| async move {
        let model = connect_model(config_dir, &reference).await?;
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(secs), model.call(request))
                .await
                .map_err(|_| "The provider did not answer in time.".to_string())?
                .map_err(|e| crate::chat::context::short_error(&e.to_string()))?;
        let text = response.text();
        // A thinking model can spend a small budget on thought and say nothing.
        if text.trim().is_empty() {
            return Err("The model gave no text back.".to_string());
        }
        Ok::<String, String>(text)
    };
    match attempt(reference.to_string(), request.clone()).await {
        Ok(text) => Ok(text),
        Err(first) => match providers::load(config_dir)
            .backup_model
            .filter(|b| b != reference)
        {
            Some(backup) => {
                log::warn!("{reference} failed ({first}); trying the backup model {backup}");
                attempt(backup, request).await.map_err(|_| first)
            }
            None => Err(first),
        },
    }
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
    run_turn_on(Arc::new(TauriHost::new(app)), request, cancel).await
}

/// [`run_turn`] on any [`Host`].
pub(crate) async fn run_turn_on(
    host: Arc<dyn Host>,
    request: TurnRequest,
    cancel: watch::Receiver<bool>,
) {
    let id = request.conversation_id.clone();
    host.emit(ChatEvent::Started {
        conversation_id: id.clone(),
    });
    let request = match auto_route(host.as_ref(), request) {
        Ok(r) => r,
        Err(message) => {
            host.chat_state().finish(&id);
            host.emit(ChatEvent::Error {
                conversation_id: id,
                message,
            });
            return;
        }
    };
    let request = route_to_agent(host.as_ref(), request).await;
    let outcome = turn(&host, &request, cancel).await;
    host.chat_state().finish(&id);
    if matches!(outcome, Ok(false)) {
        let (host, id) = (host.clone(), id.clone());
        tauri::async_runtime::spawn(async move { learn_from_turn(host.as_ref(), &id).await });
    }
    if let Ok(cancelled) = outcome {
        // The user's turn_end hooks, without holding up the next message.
        if let Ok(dir) = host.config_dir() {
            if crate::chat::hooks::load(&dir)
                .iter()
                .any(|h| h.enabled && h.event == "turn_end")
            {
                let id = id.clone();
                tauri::async_runtime::spawn(async move {
                    let Ok(c) = store::load_conversation(&dir, &id) else {
                        return;
                    };
                    let answer = store::turns(&c)
                        .iter()
                        .rev()
                        .find(|t| t.role == "assistant")
                        .map(|t| {
                            t.parts
                                .iter()
                                .filter_map(|p| {
                                    if let store::Part::Text { text } = p {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    crate::chat::hooks::turn_end(
                        &dir,
                        &id,
                        &answer,
                        cancelled,
                        c.workdir.as_deref().map(std::path::Path::new),
                    )
                    .await;
                });
            }
        }
    }
    // Filters see the finished answer too, and can rewrite it before it is shown for good.
    if matches!(outcome, Ok(false)) {
        if let Ok(dir) = host.config_dir() {
            if crate::chat::functions::has_filters(&dir) {
                filter_answer(&dir, &id).await;
            }
        }
    }
    match outcome {
        Ok(cancelled) => host.emit(ChatEvent::Done {
            conversation_id: id,
            cancelled,
        }),
        Err(message) => {
            warn!("chat turn failed: {message}");
            // A conversation whose first reply failed still gets a name in the sidebar.
            if let Ok(dir) = host.config_dir() {
                if let Ok(mut c) = store::load_conversation(&dir, &id) {
                    if c.title.is_empty() && !c.messages.is_empty() {
                        c.title = fallback_title(&c);
                        let title = c.title.clone();
                        if store::save_conversation(&dir, &c).is_ok() {
                            host.emit(ChatEvent::Title {
                                conversation_id: id.clone(),
                                title,
                            });
                        }
                    }
                }
            }
            // A provider's raw error (status, JSON) becomes what it means; the log keeps it all.
            let message =
                if message.contains("ProviderResponseError") || message.contains("status ") {
                    crate::chat::context::short_error(&message)
                } else {
                    message
                };
            host.emit(ChatEvent::Error {
                conversation_id: id,
                message,
            });
        }
    }
}

type QueuedTurn = (
    Arc<dyn Host>,
    TurnRequest,
    watch::Receiver<bool>,
    tokio::sync::oneshot::Sender<()>,
);

/// Runs a turn from inside another one (an agent messaging an agent). The turn goes through a
/// queue to a task of its own, since a turn cannot spawn its own kind directly.
fn queue_turn(
    host: &Arc<dyn Host>,
    request: TurnRequest,
    cancel: watch::Receiver<bool>,
) -> tokio::sync::oneshot::Receiver<()> {
    static QUEUE: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<QueuedTurn>> =
        std::sync::OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<QueuedTurn>();
        tauri::async_runtime::spawn(async move {
            while let Some((host, request, cancel, done)) = rx.recv().await {
                tauri::async_runtime::spawn(async move {
                    run_turn_on(host, request, cancel).await;
                    let _ = done.send(());
                });
            }
        });
        tx
    });
    let (done, wait) = tokio::sync::oneshot::channel();
    let _ = queue.send((host.clone(), request, cancel, done));
    wait
}

static MESSAGE_DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `message_agent`: the receiving agent answers in a new chat of its own (unread, so the user
/// sees it), and its reply comes back as the tool's result.
async fn message_agent(
    host: &Arc<dyn Host>,
    config_dir: &std::path::Path,
    from_chat: &str,
    from: Option<&agents::Agent>,
    arguments: &Value,
    workdir: Option<&str>,
    approval: ApprovalMode,
) -> tools::Outcome {
    use std::sync::atomic::Ordering;
    let fail = |text: String| tools::Outcome {
        text,
        is_error: true,
    };
    let s = |k: &str| {
        arguments
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let team = agents::load(config_dir);
    let Some(target) = agents::by_name(&team, &s("agent")).cloned() else {
        let names: Vec<&str> = team.iter().map(|a| a.name.as_str()).collect();
        return fail(format!(
            "There is no agent by that name. The agents are: {}.",
            names.join(", ")
        ));
    };
    if from.is_some_and(|f| f.id == target.id) {
        return fail("An agent cannot message itself.".into());
    }
    if s("message").is_empty() {
        return fail("Write the message.".into());
    }
    let paths: Vec<String> = arguments
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let files = match agents::read_shared(&paths, workdir.map(std::path::Path::new)) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    if MESSAGE_DEPTH.fetch_add(1, Ordering::SeqCst) >= agents::MAX_MESSAGE_DEPTH {
        MESSAGE_DEPTH.fetch_sub(1, Ordering::SeqCst);
        return fail(
            "Messages are already going back and forth; answer with what you have.".into(),
        );
    }
    let sender = from.map_or_else(|| "the assistant".to_string(), |a| a.name.clone());
    let model = target
        .model
        .clone()
        .or_else(|| {
            store::load_conversation(config_dir, from_chat)
                .ok()
                .and_then(|c| c.model)
        })
        .or(providers::load(config_dir).default_model);
    let result = async {
        let model = model.ok_or("No model to run the other agent on.")?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut c = Conversation::new(id.clone(), None, Some(model.clone()));
        c.title = format!(
            "From {sender}: {}",
            s("message").chars().take(50).collect::<String>()
        );
        c.agent_id = Some(target.id.clone());
        c.unread = true;
        store::save_conversation(config_dir, &c)?;
        let cancel = host.chat_state().begin(&id).ok_or("That chat is busy.")?;
        let request = TurnRequest {
            conversation_id: id.clone(),
            text: agents::message_text(&sender, &s("message"), &files),
            model,
            speed: if target.think {
                Speed::Think
            } else {
                Speed::Fast
            },
            // The sender's mode: the same person is (or is not) there to approve. From a chat in
            // Auto the other agent runs its code and makes its changes; from Ask it asks; from a
            // scheduled run nobody watches it stays unattended.
            approval,
            project_id: None,
            agent_id: Some(target.id.clone()),
            images: Vec::new(),
            edit_index: None,
            temporary: false,
            resume: false,
            workdir: None,
            connectors: None,
            route_note: None,
            tools_off: Vec::new(),
            max_steps: None,
            only_tools: None,
            system: None,
            skills_off: Vec::new(),
            facts_off: Vec::new(),
            pages_off: Vec::new(),
            instructions_off: false,
        };
        // A turn of its own, so the other agent's events stream to its chat as usual.
        queue_turn(host, request, cancel)
            .await
            .map_err(|_| "The other agent's turn stopped.".to_string())?;
        let done = store::load_conversation(config_dir, &id)?;
        let reply = done
            .messages
            .last()
            .filter(|m| matches!(m, Message::Assistant(_)))
            .map(crate::chat::context::message_text)
            .unwrap_or_default();
        if reply.trim().is_empty() {
            return Err(format!("{} could not answer.", target.name));
        }
        Ok::<String, String>(format!(
            "{} replied:\n\n{reply}\n\n(The exchange is kept in a chat under {} in the sidebar.)",
            target.name, target.name
        ))
    }
    .await;
    MESSAGE_DEPTH.fetch_sub(1, Ordering::SeqCst);
    match result {
        Ok(text) => tools::Outcome {
            text,
            is_error: false,
        },
        Err(e) => fail(e),
    }
}

/// Skills from work (learning.rs): after an agent's turn with several tool steps, a reflection
/// may turn the procedure into a skill, saved turned off; the chat says so.
async fn learn_from_turn(host: &dyn Host, id: &str) {
    let Ok(dir) = host.config_dir() else { return };
    if !crate::chat::learning::load(&dir).skills_from_work {
        return;
    }
    let Ok(c) = store::load_conversation(&dir, id) else {
        return;
    };
    let Some(agent) = c.agent_id.as_deref().and_then(|a| agents::find(&dir, a)) else {
        return;
    };
    let turns = store::turns(&c);
    let Some(last) = turns.iter().rev().find(|t| t.role == "assistant") else {
        return;
    };
    let steps: Vec<String> = last
        .parts
        .iter()
        .filter_map(|p| match p {
            store::Part::Tool {
                name, arguments, ..
            } => Some(format!(
                "{name}: {}",
                arguments.to_string().chars().take(160).collect::<String>()
            )),
            _ => None,
        })
        .collect();
    if steps.len() < crate::chat::learning::MIN_STEPS {
        return;
    }
    let request = turns
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map(|t| {
            t.parts
                .iter()
                .filter_map(|p| {
                    if let store::Part::Text { text } = p {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let answer: String = last
        .parts
        .iter()
        .filter_map(|p| {
            if let store::Part::Text { text } = p {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let existing: Vec<String> = crate::chat::skills::list(&dir)
        .into_iter()
        .map(|s| s.name)
        .collect();
    let reference = crate::chat::router::models(&providers::load(&dir))
        .map(|(fast, _)| fast)
        .or_else(|| c.model.clone())
        .or(providers::load(&dir).default_model);
    let Some(reference) = reference else { return };
    let prompt =
        crate::chat::learning::reflect_prompt(&agent.name, &request, &steps, &answer, &existing);
    let reply = match ask_text(
        &dir,
        &reference,
        CompletionRequest::new(Message::user(prompt)),
        60,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("could not reflect on {}'s work: {e}", agent.name);
            return;
        }
    };
    let Some(skill) = crate::chat::learning::parse_reflection(&reply, &existing) else {
        return;
    };
    if let Err(e) = crate::chat::learning::keep(&dir, &agent.name, &skill) {
        warn!("could not keep a learned skill: {e}");
        return;
    }
    info!("{} learned the skill {}", agent.name, skill.name);
    let note = format!(
        "{} learned a skill from this work: /{}, turned off until you review it under Skills.",
        agent.name, skill.name
    );
    if let Ok(mut c) = store::load_conversation(&dir, id) {
        c.notices.push((c.messages.len(), note.clone()));
        if let Err(e) = store::save_conversation(&dir, &c) {
            log::warn!("could not save a notice to conversation {id}: {e}");
        }
    }
    host.emit(ChatEvent::Notice {
        conversation_id: id.into(),
        text: note,
    });
}

/// The orchestrator: a new Work chat goes to the agent that fits it best, when that is turned on
/// and one clearly fits; the chat says so. Any failure leaves the chat as it was.
async fn route_to_agent(host: &dyn Host, mut request: TurnRequest) -> TurnRequest {
    let Ok(dir) = host.config_dir() else {
        return request;
    };
    let fresh = store::load_conversation(&dir, &request.conversation_id).is_err();
    if !fresh
        || request.agent_id.is_some()
        || request.workdir.is_some()
        || request.text.trim().is_empty()
        || !agents::routing_enabled(&dir)
    {
        return request;
    }
    let team = agents::load(&dir);
    if team.is_empty() {
        return request;
    }
    // The fast model decides when Auto has one; otherwise the chat's own.
    let reference = crate::chat::router::models(&providers::load(&dir))
        .map_or_else(|| request.model.clone(), |(fast, _)| fast);
    let ask = CompletionRequest::new(Message::user(agents::route_prompt(&team, &request.text)));
    let reply = match ask_text(&dir, &reference, ask, 30).await {
        Ok(r) => r,
        Err(e) => {
            warn!("could not route the chat to an agent: {e}");
            return request;
        }
    };
    if let Some(agent) = agents::parse_route(&team, &reply) {
        info!("routed a new chat to {}", agent.name);
        request.agent_id = Some(agent.id.clone());
        let note = format!("Cortex handed this chat to {}.", agent.name);
        request.route_note = Some(match request.route_note.take() {
            Some(before) => format!("{before} {note}"),
            None => note,
        });
    }
    request
}

/// The Auto model: picks the fast or the strong model for this message and says which.
fn auto_route(host: &dyn Host, mut request: TurnRequest) -> Result<TurnRequest, String> {
    if request.model != crate::chat::router::AUTO {
        return Ok(request);
    }
    let dir = host.config_dir()?;
    let (fast, strong) = crate::chat::router::models(&providers::load(&dir))
        .ok_or("Choose Auto's fast and strong models under Settings → Providers & models first.")?;
    // Regenerating: the message being answered again decides.
    let text = if request.text.trim().is_empty() {
        store::load_conversation(&dir, &request.conversation_id)
            .ok()
            .and_then(|c| store::last_prompt(&c).and_then(|i| c.messages.get(i).cloned()))
            .map(|m| match m {
                Message::User { content } => content
                    .iter()
                    .filter_map(|c| {
                        if let rig_core::completion::message::UserContent::Text(t) = c {
                            Some(t.text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            })
            .unwrap_or_default()
    } else {
        request.text.clone()
    };
    let route = crate::chat::router::route(&text, request.images.len());
    request.model = if route.strong { strong } else { fast };
    let name = providers::split_reference(&request.model)
        .map_or(request.model.as_str(), |(_, m)| m)
        .to_string();
    request.route_note = Some(format!("Auto chose {name}: {}.", route.reason));
    Ok(request)
}

pub const READ_TOOL_OUTPUT: &str = "read_tool_output";

fn tool_output_path(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
) -> Option<std::path::PathBuf> {
    let safe = |s: &str| {
        !s.is_empty()
            && s.len() <= 120
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && !s.starts_with('.')
    };
    (safe(conversation) && safe(call_id)).then(|| {
        store::chat_dir(config_dir)
            .join("tool-outputs")
            .join(conversation)
            .join(format!("{call_id}.txt"))
    })
}

/// Keeps a long tool output in `chat/tool-outputs/<conversation>/<call>.txt` (Vibe 3.0.1).
fn save_tool_output(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
    text: &str,
) -> Result<(), String> {
    let path = tool_output_path(config_dir, conversation, call_id).ok_or("unusable call id")?;
    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())
}

/// A part of a saved long output, from `offset` characters, at most 20,000.
pub fn read_tool_output(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
    offset: usize,
) -> Result<String, String> {
    let path = tool_output_path(config_dir, conversation, call_id)
        .ok_or("There is no saved output with that call id.")?;
    let text = crate::chat::at_rest::read_to_string(path).map_err(|_| {
        "There is no saved output with that call id in this conversation.".to_string()
    })?;
    let total = text.chars().count();
    let part: String = text.chars().skip(offset).take(20_000).collect();
    let end = (offset + part.chars().count()).min(total);
    Ok(format!(
        "[characters {offset}–{end} of {total}]\n{part}{}",
        if end < total {
            format!("\n[… continue with offset={end}]")
        } else {
            String::new()
        }
    ))
}

fn read_tool_output_tool() -> ChatTool {
    ChatTool {
        name: READ_TOOL_OUTPUT.into(),
        description: "Read a part of a long tool result that was shown only in part: give the call id and the offset the result named.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "call_id": { "type": "string" },
                "offset": { "type": "integer", "minimum": 0 }
            },
            "required": ["call_id"]
        }),
        connector_id: None,
        tool_name: READ_TOOL_OUTPUT.into(),
        read_only: true,
    }
}

/// What a back end that needs the desktop app says where there is none.
const NO_APP: &str = "This model runs only in the desktop app.";

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

/// Runs the outlet filters over the last answer; a changed answer replaces it.
async fn filter_answer(config_dir: &std::path::Path, id: &str) {
    use rig_core::completion::message::AssistantContent;
    let Ok(mut c) = store::load_conversation(config_dir, id) else {
        return;
    };
    let Some(Message::Assistant(last)) = c.messages.last() else {
        return;
    };
    if last
        .content
        .iter()
        .any(|p| matches!(p, AssistantContent::ToolCall(_)))
    {
        return;
    }
    let text = last
        .content
        .iter()
        .filter_map(|p| {
            if let AssistantContent::Text(t) = p {
                Some(t.text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    if text.trim().is_empty() {
        return;
    }
    let model = c.model.clone().unwrap_or_default();
    match crate::chat::functions::apply(config_dir, id, "outlet", &model, &text).await {
        Ok(changed) if changed != text => {
            if let Some(last) = c.messages.last_mut() {
                *last = Message::assistant(changed);
            }
            if let Err(e) = store::save_conversation(config_dir, &c) {
                log::warn!("could not save the filtered answer: {e}");
            }
        }
        Ok(_) => {}
        Err(reason) => {
            if let Some(last) = c.messages.last_mut() {
                *last = Message::assistant(format!("_The answer was withheld. {reason}_"));
            }
            if let Err(e) = store::save_conversation(config_dir, &c) {
                log::warn!("could not save the filtered answer: {e}");
            }
        }
    }
}

/// Returns whether the turn was cancelled. Every turn is traced (see [`crate::chat::traces`]).
pub(crate) async fn turn(
    host: &Arc<dyn Host>,
    request: &TurnRequest,
    cancel: watch::Receiver<bool>,
) -> Result<bool, String> {
    let run = uuid::Uuid::new_v4().to_string();
    let outcome = traced_turn(host, request, cancel, &run).await;
    if let Ok(dir) = host.config_dir() {
        let ending = match &outcome {
            Ok(false) => "done",
            Ok(true) => "cancelled",
            Err(_) => "error",
        };
        crate::chat::traces::end_run(&dir, &run, ending);
    }
    outcome
}

async fn traced_turn(
    host: &Arc<dyn Host>,
    request: &TurnRequest,
    mut cancel: watch::Receiver<bool>,
    run: &str,
) -> Result<bool, String> {
    let config_dir = host.config_dir()?;
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
    // Filters see the message first: they can rewrite it or refuse it.
    let mut text = request.text.trim().to_string();
    if !text.is_empty() && crate::chat::functions::has_filters(&config_dir) {
        text = crate::chat::functions::apply(&config_dir, id, "inlet", &request.model, &text)
            .await
            .map_err(|reason| format!("Not sent. {reason}"))?;
    }

    if let Some(index) = request.edit_index {
        let is_prompt = conversation.messages.get(index).is_some_and(|m| {
            matches!(m, Message::User { content } if content.iter().any(|c| matches!(c, rig_core::completion::message::UserContent::Text(_))))
        });
        if !is_prompt || text.is_empty() {
            return Err("That message cannot be edited.".into());
        }
        // The old message and everything after it stay available as a branch.
        store::keep_branch(&mut conversation, index, "prompt");
        conversation.messages.truncate(index);
        conversation.times.retain(|(i, _)| *i < index);
        conversation.ratings.retain(|(i, _)| *i < index);
        if conversation
            .summary
            .as_ref()
            .is_some_and(|s| s.upto > index)
        {
            conversation.summary = None;
        }
        conversation.messages.push(Message::user(text.as_str()));
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
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
        conversation
            .messages
            .push(user_message(&text, &request.images)?);
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
    }
    conversation.updated_at = Utc::now();
    // What the user did says something about the last answer, rated or not.
    let signal = if request.edit_index.is_some() {
        Some("edited")
    } else if request.resume {
        Some("resumed")
    } else if request.text.trim().is_empty() {
        Some("regenerated")
    } else {
        None
    };
    if let Some(kind) = signal {
        crate::chat::traces::signal(&config_dir, id, request.edit_index, kind);
    }
    if let Some(note) = &request.route_note {
        conversation
            .notices
            .push((conversation.messages.len(), note.clone()));
        host.emit(ChatEvent::Notice {
            conversation_id: id.into(),
            text: note.clone(),
        });
    }
    store::save_conversation(&config_dir, &conversation)?;

    // A pipe (a function of the user's) answers with whatever its code prints.
    if let Some(pipe) = crate::chat::functions::find_pipe(&config_dir, &request.model) {
        let named = !conversation.title.is_empty();
        let args = serde_json::json!({ "messages": crate::chat::functions::messages(&conversation), "model": pipe.id });
        let answer = tokio::select! {
            out = crate::chat::functions::run(&config_dir, id, &pipe, &args) => Some(out?),
            _ = cancelled(&mut cancel) => None,
        };
        let Some(answer) = answer else {
            return Ok(true);
        };
        if answer.trim().is_empty() {
            return Err(format!("{} printed nothing.", pipe.name));
        }
        host.emit(ChatEvent::Text {
            conversation_id: id.into(),
            text: answer.clone(),
        });
        conversation.messages.push(Message::assistant(answer));
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
        conversation.updated_at = Utc::now();
        store::save_conversation(&config_dir, &conversation)?;
        if !named {
            let host = host.clone();
            let reference = request.model.clone();
            tauri::async_runtime::spawn(async move {
                name_conversation(host, reference, conversation).await
            });
        }
        return Ok(false);
    }
    // An ACP agent answers over the Agent Client Protocol, with its own tools.
    if let Some(acp_agent) = crate::chat::acp::find(&config_dir, &request.model) {
        let named = !conversation.title.is_empty();
        let cancelled = crate::chat::acp::run(
            host.app().ok_or(NO_APP)?,
            &config_dir,
            conversation.clone(),
            &acp_agent,
            cancel,
        )
        .await?;
        if !named {
            let host = host.clone();
            let reference = request.model.clone();
            tauri::async_runtime::spawn(async move {
                name_conversation(host, reference, conversation).await
            });
        }
        return Ok(cancelled);
    }
    // A coding agent program (Claude Code, Codex, Gemini CLI) answers with its own tools.
    if let Some(cli) = crate::chat::agent_cli::find(&request.model) {
        let named = !conversation.title.is_empty();
        let cancelled = crate::chat::agent_cli::run(
            host.app().ok_or(NO_APP)?,
            &config_dir,
            conversation.clone(),
            cli,
            cancel,
        )
        .await?;
        if !named {
            let host = host.clone();
            let reference = request.model.clone();
            tauri::async_runtime::spawn(async move {
                name_conversation(host, reference, conversation).await
            });
        }
        return Ok(cancelled);
    }

    let model = host.connect_model(&request.model).await?;
    let registry = host.registry();
    let agent = conversation
        .agent_id
        .as_deref()
        .and_then(|a| agents::find(&config_dir, a));
    if let Some(agent) = &agent {
        agents::check_budget(&config_dir, agent)?;
    }
    crate::chat::traces::start_run(
        &config_dir,
        run,
        id,
        agent.as_ref().map(|a| a.id.as_str()),
        &request.model,
    );
    let model_ref = request.model.clone();
    // The backup model, tried once when the main one fails before anything streamed.
    let fallback = agent
        .as_ref()
        .and_then(|a| a.fallback_model.clone())
        .or_else(|| providers::load(&config_dir).backup_model)
        .filter(|f| f != &request.model);
    let mut offered = tools::available(registry, request.connectors.as_deref());
    offered.splice(1..1, builtins::definitions(&config_dir));
    offered.push(read_tool_output_tool());
    offered.extend(crate::chat::openapi::tools(&config_dir));
    offered.extend(crate::chat::custom_tools::definitions(&config_dir));
    // Tools the assistant wrote and the user approved, and the way to write more.
    offered.extend(crate::chat::script_tools::definitions(&config_dir));
    offered.extend(crate::chat::script_tools::builtin_definitions());
    if !conversation.temporary {
        offered.push(crate::chat::proposals::definition());
    }
    if crate::chat::computer::load(&config_dir).enabled {
        offered.push(crate::chat::computer::definition());
    }
    if let Some(agent) = &agent {
        offered.retain(|t| t.name == ASK_USER || agents::allows(agent, &t.name));
    }
    // The composer's Tools menu: what the user turned off for this chat.
    if !request.tools_off.is_empty() {
        offered.retain(|t| {
            !request
                .tools_off
                .iter()
                .any(|off| off == &t.name || t.name.starts_with(&format!("{off}__")))
        });
    }
    if let Some(only) = &request.only_tools {
        offered.retain(|t| only.contains(&t.name));
    }
    if request.approval.unattended() {
        // Nobody can answer a question during a scheduled run.
        offered.retain(|t| t.name != ASK_USER);
    }
    if agent.is_some() {
        // An agent keeps a notebook across its conversations and runs.
        offered.push(agents::notebook_tool());
    }
    if agent.as_ref().is_some_and(|a| a.can_manage_agents) {
        // A manager, as the user allowed, builds and runs its own organization of agents.
        offered.push(crate::chat::org::definition());
    }
    if conversation.workdir.is_some() {
        // Coding sessions get a terminal for long-running commands.
        offered.extend(terminal::definitions());
    }
    if conversation.temporary {
        // A temporary chat leaves no trace: it does not add to or change knowledge.
        // Nor the assistant's own tools and skills: an eval case or a worker runs in one.
        offered.retain(|t| {
            !matches!(
                t.name.as_str(),
                builtins::REMEMBER
                    | builtins::FORGET
                    | builtins::KNOWLEDGE_WRITE
                    | builtins::SAVE_SKILL
                    | crate::chat::script_tools::CREATE_TOOL
                    | crate::chat::script_tools::PROMOTE_TOOL
                    | crate::chat::script_tools::RETIRE_TOOL
                    | crate::chat::script_tools::REQUEST_CAPABILITY
            )
        });
    }
    // Past a few dozen connector tools, the rest load on demand (Vibe's "Found tools").
    let defer = deferred::should_defer(&offered);
    if defer {
        offered.push(deferred::find_tools_tool());
        offered.push(handlers::unload_tools_tool());
    }
    offered.push(handlers::context_status_tool());
    let project = conversation.project_id.as_ref().and_then(|pid| {
        store::load_projects(&config_dir)
            .into_iter()
            .find(|p| &p.id == pid)
    });
    let instructions = if request.instructions_off {
        store::Instructions::default()
    } else {
        store::load_instructions(&config_dir)
    };
    let mut memories = memory::load(&config_dir);
    memories
        .items
        .retain(|m| !request.facts_off.contains(&m.id));
    let mut pages = knowledge::load(&config_dir);
    pages.retain(|p| !request.pages_off.contains(&p.id));
    let mut skill_list = skills::list(&config_dir);
    if let Some(agent) = agent.as_ref().filter(|a| !a.skills.is_empty()) {
        skill_list.retain(|s| agent.skills.contains(&s.name));
    }
    skill_list.retain(|s| !request.skills_off.contains(&s.name));
    let listed_skills: Vec<String> = skill_list
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.clone())
        .collect();
    crate::chat::traces::loaded(&config_dir, "skill", &listed_skills);
    for name in &listed_skills {
        crate::chat::traces::attribute(&config_dir, run, "skill", name, false);
    }
    let canvases = canvas::list_for(&config_dir, id);
    let latest_prompt = store::last_prompt(&conversation)
        .and_then(|i| conversation.messages.get(i))
        .map(crate::chat::context::message_text)
        .unwrap_or_default();
    attribute_loaded(
        &config_dir,
        run,
        &offered,
        &memories,
        &pages,
        &latest_prompt,
        project.as_ref().map(|p| p.id.as_str()),
        agent.as_ref().map(|a| a.id.as_str()),
    );
    let library_list = libraries::list(&config_dir);
    let mut prompt = preamble_prompt(
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
            query: &latest_prompt,
        },
        &offered,
    );
    use crate::chat::prompt::{ESSENTIAL, OPTIONAL, USEFUL};
    // A built-in model runs on this computer's processor: it gets a short brief and no tools, so
    // it starts answering in seconds rather than reading thousands of tokens of tool manuals.
    if request.model.starts_with("local/") {
        offered.clear();
        prompt.replace_all(
            "brief",
            crate::chat::local_models::brief(
                &instructions,
                project.as_ref().map(|p| p.instructions.as_str()),
                agent
                    .as_ref()
                    .map(|a| (a.name.as_str(), a.instructions.as_str())),
            ),
        );
    }
    if let Some(secrets) = crate::chat::vault::prompt(&crate::chat::vault::granted(
        &crate::chat::vault::list(&config_dir),
        agent.as_ref(),
    )) {
        prompt.push("vault", ESSENTIAL, secrets);
    }
    if !conversation.params.instructions.is_empty() {
        prompt.push(
            "chat instructions",
            ESSENTIAL,
            format!(
                "\n\n## Instructions for this conversation\n{}",
                conversation.params.instructions
            ),
        );
    }
    if defer {
        prompt.push(
            "tools to load",
            ESSENTIAL,
            deferred::prompt(&offered, &conversation.loaded_tools),
        );
    }
    if let Some(agent) = &agent {
        prompt.push(
            "notebook",
            ESSENTIAL,
            agents::notebook_prompt(&agents::notebook(&config_dir, &agent.id)),
        );
        if agent.can_manage_agents {
            prompt.push(
                "organization",
                USEFUL,
                crate::chat::org::prompt(&agents::load(&config_dir), agent),
            );
        }
    }
    if offered.iter().any(|t| t.name == builtins::DELEGATE) {
        let all = agents::load(&config_dir);
        // A group hosts its members; any other chat may call on the whole team.
        match agent.as_ref().and_then(|a| agents::group_prompt(&all, a)) {
            Some(group) => prompt.push("group", USEFUL, group),
            None => {
                if let Some(team) = agents::team_prompt(&all, agent.as_ref().map(|a| a.id.as_str()))
                {
                    prompt.push("team", OPTIONAL, team);
                }
            }
        }
    }
    // Knowledge pages that matter for this message come in full; the rest by title.
    if memories.enabled {
        if let Some(section) = knowledge::prompt(&pages, &latest_prompt) {
            prompt.push("knowledge pages", OPTIONAL, section);
        }
    }
    if request.approval == ApprovalMode::Plan {
        prompt.push(
            "plan mode",
            ESSENTIAL,
            "\n\nPlan mode is on. Investigate with read-only tools, then present a numbered plan \
             of the changes you would make (files, commands, risks) and stop. Do not try to change \
             anything; the user approves the plan and then switches to an editing mode.",
        );
    }
    if let Some(brief) = &request.system {
        prompt.replace_all("brief", brief.clone());
    }
    let reasoning = providers::supports_reasoning(&request.model);
    let window = context::context_window(&request.model);
    // Within its budget, the least useful sections give way; the context view shows what stayed.
    let (system, parts, dropped) = prompt.fit(window);
    if !dropped.is_empty() {
        info!(
            "left out of the prompt to fit the window: {}",
            dropped.join(", ")
        );
    }
    conversation.prompt_parts = parts;
    // Personal data in what the user typed is swapped for placeholders before a cloud request.
    let filter = privacy::load(&config_dir).enabled
        && providers::split_reference(&request.model)
            .is_some_and(|(vendor, _)| privacy::is_cloud(vendor));
    let redactions = privacy::Redactions::default();
    // Checked before every step, not only the first: tool results can fill the window within one
    // turn. A failed summary is not retried in the same turn, so it costs one call at most.
    let compaction_failed = false;
    let max_steps = request.max_steps.unwrap_or(MAX_STEPS).max(1);

    let mut steps = TurnSteps {
        host,
        request,
        cancel,
        conversation,
        model,
        model_ref,
        fallback,
        redactions,
        compaction_failed,
        system,
        offered,
        defer,
        agent,
        config_dir,
        id,
        window,
        filter,
        reasoning,
        run,
        calls: Vec::new(),
    };
    if cortex_kernel::agent_loop::run(&mut steps, max_steps).await? {
        return Ok(true);
    }
    let conversation = steps.conversation;

    if conversation.title.is_empty() {
        let host = host.clone();
        let reference = request.model.clone();
        tauri::async_runtime::spawn(async move {
            name_conversation(host, reference, conversation).await
        });
    }
    Ok(false)
}

/// A turn's state across its steps, for the kernel's loop ([`cortex_kernel::agent_loop`]): each
/// step streams one model call, then runs the tools it called.
struct TurnSteps<'a> {
    host: &'a Arc<dyn Host>,
    request: &'a TurnRequest,
    cancel: watch::Receiver<bool>,
    conversation: Conversation,
    model: DynModel<Completion>,
    model_ref: String,
    fallback: Option<String>,
    redactions: privacy::Redactions,
    compaction_failed: bool,
    system: String,
    offered: Vec<ChatTool>,
    defer: bool,
    agent: Option<agents::Agent>,
    config_dir: std::path::PathBuf,
    id: &'a str,
    window: u64,
    filter: bool,
    reasoning: bool,
    run: &'a str,
    /// The tool calls of the step just made, for `run_calls`.
    calls: Vec<ToolCall>,
}

#[async_trait::async_trait]
impl Steps for TurnSteps<'_> {
    async fn call_model(&mut self, step: usize, last: bool) -> Result<Reply, String> {
        let TurnSteps {
            host,
            request,
            cancel,
            conversation,
            model,
            model_ref,
            fallback,
            redactions,
            compaction_failed,
            system,
            offered,
            defer,
            agent,
            config_dir,
            id,
            window,
            filter,
            reasoning,
            run,
            calls: pending,
        } = self;
        let (host, request, id, run) = (*host, *request, *id, *run);
        let (defer, window, filter, reasoning) = (*defer, *window, *filter, *reasoning);
        let config_dir = config_dir.as_path();
        // Guidance the user sent while this turn was running joins the conversation here.
        for text in host.chat_state().take_steering(id) {
            conversation.messages.push(Message::user(format!(
                "(Sent while you were working) {text}"
            )));
            conversation
                .times
                .push((conversation.messages.len() - 1, Utc::now()));
            host.emit(ChatEvent::Steered {
                conversation_id: id.into(),
                text,
            });
        }
        if !*compaction_failed {
            *compaction_failed =
                compact_if_needed(host.as_ref(), model, conversation, window, config_dir).await
                    == Compaction::Failed;
        }
        let mut carried =
            context::request_messages(&conversation.messages, conversation.summary.as_ref());
        if filter {
            carried = privacy::redact_messages(&carried, redactions);
        }
        let Some((prompt, history)) = carried.split_last() else {
            return Err("empty conversation".into());
        };
        let last_step = last;
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
        // An agent's reminder rides on the newest message the user typed, every request; it is
        // not saved with the conversation.
        let mut prompt = prompt.clone();
        if let Some(reminder) = agent
            .as_ref()
            .map(|a| a.reminder.trim())
            .filter(|r| !r.is_empty())
        {
            if let Message::User { content } = &mut prompt {
                if let Some(rig_core::completion::message::UserContent::Text(t)) = content
                    .iter_mut()
                    .find(|c| matches!(c, rig_core::completion::message::UserContent::Text(_)))
                {
                    t.text = format!("{}\n\n(Reminder: {reminder})", t.text);
                }
            }
        }
        let mut call = CompletionRequest::new(prompt)
            .messages(history.iter().cloned())
            .preamble(step_system);
        if !last_step {
            // Deferred tools go out once they are loaded; the rest every time.
            let definitions = offered
                .iter()
                .filter(|t| {
                    !defer
                        || !deferred::deferrable(t)
                        || conversation.loaded_tools.contains(&t.name)
                })
                .filter_map(ChatTool::definition);
            call = call.tools(definitions);
        }
        // This conversation's own model settings, then the app-wide defaults.
        let params = conversation
            .params
            .or_defaults(&store::chat_defaults(config_dir));
        if let Some(t) = params.temperature {
            call = call.temperature(t);
        }
        if let Some(n) = params.max_tokens {
            call = call.max_tokens(n);
        }
        if let Some(p) = params.top_p {
            call = call.top_p(p);
        }
        if let Some(effort) = request.speed.effort().filter(|_| reasoning) {
            call = call.reasoning(effort);
        }

        // Transient failures (dropped connections, rate limits, overloaded servers) are retried
        // with backoff, as long as nothing reached the user yet.
        let mut attempt = 0u32;
        let call_started = std::time::Instant::now();
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
                    _ = cancelled(cancel) => break true,
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
                                let text = if filter { restore_streamed(redactions, &mut held, &text) } else { text };
                                streamed_text.push_str(&text);
                                if !text.is_empty() {
                                    host.emit(ChatEvent::Text { conversation_id: id.into(), text });
                                }
                            }
                            StreamEvent::Reasoning { text, .. } => {
                                streamed_anything = true;
                                thinking.get_or_insert((std::time::Instant::now(), None));
                                host.emit(ChatEvent::Reasoning { conversation_id: id.into(), text });
                            }
                            StreamEvent::Start { kind: PartKind::ToolCall, name: Some(name), .. } => {
                                streamed_anything = true;
                                if let Some((_, end @ None)) = &mut thinking {
                                    *end = Some(std::time::Instant::now());
                                }
                                host.emit(ChatEvent::ToolPending { conversation_id: id.into(), name: name.to_string() });
                            }
                            _ => {}
                        },
                    },
                }
            };

            if was_cancelled {
                if !streamed_text.trim().is_empty() {
                    conversation
                        .messages
                        .push(Message::assistant(streamed_text));
                    conversation
                        .times
                        .push((conversation.messages.len() - 1, Utc::now()));
                }
                conversation.updated_at = Utc::now();
                store::save_conversation(config_dir, conversation)?;
                return Ok(Reply::Cancelled);
            }

            thought_ms = thinking.map(|(start, end)| {
                end.unwrap_or_else(std::time::Instant::now)
                    .duration_since(start)
                    .as_millis()
                    .min(u32::MAX as u128) as u32
            });
            let finished = stream.finish().await;
            if let Err(e) = &finished {
                crate::chat::traces::model_call(
                    config_dir,
                    run,
                    step,
                    model_ref,
                    call_started.elapsed().as_millis() as u64,
                    None,
                    Some(if e.is_retryable() {
                        "transient"
                    } else {
                        "failed"
                    }),
                );
            }
            match finished {
                Ok(response) => {
                    crate::chat::traces::model_call(
                        config_dir,
                        run,
                        step,
                        model_ref,
                        call_started.elapsed().as_millis() as u64,
                        Some(&response.usage),
                        None,
                    );
                    break response;
                }
                Err(e) if e.is_retryable() && !streamed_anything && attempt < MAX_ATTEMPTS => {
                    let wait = std::time::Duration::from_secs(2u64.pow(attempt));
                    warn!("model call failed ({e}); retrying in {}s", wait.as_secs());
                    tokio::select! {
                        _ = cancelled(cancel) => return Ok(Reply::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
                Err(e) if !streamed_anything && fallback.is_some() => {
                    // Fail over to the agent's backup model, once, and say so.
                    let backup = fallback.take().unwrap_or_default();
                    warn!("model call failed ({e}); switching to {backup}");
                    match host.connect_model(&backup).await {
                        Ok(next) => {
                            *model = next;
                            model_ref.clone_from(&backup);
                            attempt = 0;
                            let label = backup.rsplit('/').next().unwrap_or(&backup).to_string();
                            let text = format!(
                                "{}; continuing with {label}.",
                                crate::chat::context::brief_error(&e.to_string())
                            );
                            conversation
                                .notices
                                .push((conversation.messages.len(), text.clone()));
                            host.emit(ChatEvent::Notice {
                                conversation_id: id.into(),
                                text,
                            });
                        }
                        Err(_) => {
                            store::save_conversation(config_dir, conversation)?;
                            return Err(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    store::save_conversation(config_dir, conversation)?;
                    return Err(e.to_string());
                }
            }
        };
        conversation.usage.add(&response.usage);
        host.emit(ChatEvent::Usage {
            conversation_id: id.into(),
            usage: conversation.usage.clone(),
            context_window: window,
        });
        let message = response.message().map(|m| {
            if filter {
                privacy::restore_message(m, redactions)
            } else {
                m
            }
        });
        let calls: Vec<ToolCall> = match &message {
            Some(Message::Assistant(a)) => a
                .content
                .iter()
                .filter_map(|c| match c {
                    rig_core::completion::message::AssistantContent::ToolCall(call) => {
                        Some(call.clone())
                    }
                    _ => None,
                })
                .collect(),
            _ => response.tool_calls().cloned().collect(),
        };
        if let Some(message) = message {
            conversation.messages.push(message);
            if let Some(ms) = thought_ms {
                conversation
                    .thought
                    .push((conversation.messages.len() - 1, ms));
            }
        }
        conversation.updated_at = Utc::now();

        if calls.is_empty() {
            conversation
                .times
                .push((conversation.messages.len() - 1, Utc::now()));
            store::save_conversation(config_dir, conversation)?;
            return Ok(Reply::Answered);
        }
        store::save_conversation(config_dir, conversation)?;
        info!("chat step {step}: {} tool call(s)", calls.len());
        *pending = calls;
        Ok(Reply::CalledTools)
    }

    async fn run_calls(&mut self, _step: usize) -> Result<bool, String> {
        let TurnSteps {
            host,
            request,
            cancel,
            conversation,
            model,
            offered,
            agent,
            config_dir,
            id,
            run,
            calls: pending,
            ..
        } = self;
        let (host, request, id, run) = (*host, *request, *id, *run);
        let config_dir = config_dir.as_path();
        let calls = std::mem::take(pending);
        for call in &calls {
            host.emit(ChatEvent::ToolStart {
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
                || (agent.as_ref().is_some_and(|a| agents::asks_first(a, name))
                    && !session_allowed(id, name))
                || offered.iter().find(|t| t.name == name).is_some_and(|tool| {
                    tool.connector_id.is_some()
                        && tools::needs_approval(
                            tool,
                            &Value::Object(call.function.arguments.clone()),
                            agents::mode_for(agent.as_ref(), name, request.approval),
                        )
                })
        };
        // Decided once: an approval given during the step ("Always allow") must not move a call
        // out of the sequential queue after it was kept out of the concurrent batch.
        let sequential: Vec<bool> = calls.iter().map(waits_on_user).collect();
        let mut outcomes: Vec<Option<tools::Outcome>> = (0..calls.len()).map(|_| None).collect();
        let runner = Runner {
            host,
            run,
            config_dir,
            offered: offered.as_slice(),
            model: &*model,
            request,
            id,
            agent: agent.as_ref(),
            workdir: conversation.workdir.clone(),
        };
        let concurrent = futures::future::join_all(
            calls
                .iter()
                .enumerate()
                .filter(|(i, _)| !sequential[*i])
                .map(|(i, c)| {
                    let runner = &runner;
                    async move { (i, runner.run(c).await) }
                }),
        );
        let finished = tokio::select! {
            _ = cancelled(cancel) => None,
            done = concurrent => Some(done),
        };
        if let Some(done) = finished {
            for (i, outcome) in done {
                outcomes[i] = Some(outcome);
            }
        }
        for (i, call) in calls.iter().enumerate() {
            if outcomes[i].is_some() || !sequential[i] || *cancel.borrow() {
                continue;
            }
            let outcome = if call.function.name.as_str() == ASK_USER
                && request.approval.unattended()
            {
                tools::Outcome::ok("Nobody is available to answer during this scheduled run; continue with your best judgement and state your assumptions.")
            } else if call.function.name.as_str() == ASK_USER {
                ask_user(host.as_ref(), id, call, cancel).await
            } else {
                tokio::select! {
                    _ = cancelled(cancel) => cancelled_outcome(),
                    outcome = runner.run(call) => outcome,
                }
            };
            outcomes[i] = Some(outcome);
        }

        let mut results = Vec::with_capacity(calls.len());
        for (call, outcome) in calls.iter().zip(outcomes) {
            let outcome = outcome.unwrap_or_else(cancelled_outcome);
            host.emit(ChatEvent::ToolEnd {
                conversation_id: id.into(),
                call_id: call.id.to_string(),
                output: outcome.text.clone(),
                is_error: outcome.is_error,
            });
            if call.function.name.as_str() == deferred::FIND_TOOLS && !outcome.is_error {
                // What it found is loaded for the next step and the rest of the conversation.
                if let Ok(found) = serde_json::from_str::<Value>(&outcome.text) {
                    for name in found["matches"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| m["name"].as_str())
                    {
                        if !conversation.loaded_tools.iter().any(|t| t == name) {
                            conversation.loaded_tools.push(name.to_string());
                        }
                    }
                }
            }
            if call.function.name.as_str() == handlers::UNLOAD_TOOLS && !outcome.is_error {
                // Unloaded tools leave the next request; find_tools brings them back.
                if let Ok(gone) = serde_json::from_str::<Value>(&outcome.text) {
                    let gone: Vec<&str> = gone["unloaded"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    conversation
                        .loaded_tools
                        .retain(|t| !gone.contains(&t.as_str()));
                }
            }
            // A long output is kept whole in a file; the conversation keeps its start and end,
            // which is what the model reads, with how to read the rest.
            let mut text = outcome.text;
            if text.chars().count() > context::MAX_RESULT_CHARS {
                match save_tool_output(config_dir, id, &call.id.to_string(), &text) {
                    Ok(()) => text = context::head_and_tail(&text, &call.id.to_string()),
                    Err(e) => warn!("could not keep a long tool output: {e}"),
                }
            }
            let mut content = vec![ToolResultContent::text(text)];
            // Computer use: the screenshot goes with the result; older ones leave the history.
            if call.function.name.as_str() == crate::chat::computer::TOOL {
                if let Some(data) = crate::chat::computer::take_image(id) {
                    drop_old_screenshots(&mut conversation.messages);
                    content.push(ToolResultContent::image_base64(
                        data,
                        Some(rig_core::completion::message::ImageMediaType::JPEG),
                        None,
                    ));
                }
            }
            let mut result = call.result(content);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        conversation.messages.push(Message::tool_results(results));
        conversation.updated_at = Utc::now();
        store::save_conversation(config_dir, conversation)?;

        Ok(*cancel.borrow())
    }

    fn has_new_guidance(&self) -> bool {
        self.host.chat_state().has_steering(self.id)
    }
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
    tools::Outcome::err("Cancelled by the user.")
}

/// Tools the user chose "Always allow" for under an agent's ask-first policy, by conversation.
fn session_allowances() -> &'static std::sync::Mutex<std::collections::HashSet<(String, String)>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(String, String)>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

use cortex_kernel::approval::{change_gate, ChangeGate};

fn session_allowed(conversation: &str, tool: &str) -> bool {
    session_allowances()
        .lock()
        .is_ok_and(|s| s.contains(&(conversation.to_string(), tool.to_string())))
}

fn allow_for_session(conversation: &str, tool: &str) {
    if let Ok(mut s) = session_allowances().lock() {
        s.insert((conversation.to_string(), tool.to_string()));
    }
}

/// Attribution: what this run had in front of it, beyond skills: the tools offered, the facts
/// listed and the knowledge pages recalled in full. A tool called, a page read and a skill opened
/// are marked used as they happen, so a good or bad run can be traced to what it had and used.
#[allow(clippy::too_many_arguments)]
fn attribute_loaded(
    config_dir: &std::path::Path,
    run: &str,
    offered: &[tools::ChatTool],
    memories: &memory::Memories,
    pages: &[knowledge::Page],
    query: &str,
    project: Option<&str>,
    agent: Option<&str>,
) {
    for tool in offered {
        crate::chat::traces::attribute(config_dir, run, "tool", &tool.name, false);
    }
    let facts: Vec<String> = memory::shown(memories, query, project, agent)
        .into_iter()
        .map(|m| m.id)
        .collect();
    crate::chat::traces::loaded(config_dir, "memory", &facts);
    for id in &facts {
        crate::chat::traces::attribute(config_dir, run, "memory", id, false);
    }
    if memories.enabled {
        let recalled: Vec<String> = knowledge::recalled(pages, query)
            .into_iter()
            .map(|p| p.id.clone())
            .collect();
        crate::chat::traces::loaded(config_dir, "page", &recalled);
        for id in &recalled {
            crate::chat::traces::attribute(config_dir, run, "page", id, false);
        }
    }
}

/// Runs one tool call, `run`, the way every loop that calls tools must: between the user's
/// `before_tool` and `after_tool` hooks, with `agent`'s vault secrets reaching the processes it
/// starts and masked in what comes back, and recorded in the activity log.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn guarded_call<F>(
    config_dir: &std::path::Path,
    conversation_id: &str,
    trace: Option<&str>,
    agent: Option<&agents::Agent>,
    workdir: Option<&std::path::Path>,
    name: &str,
    arguments: &Value,
    run: F,
) -> tools::Outcome
where
    F: std::future::Future<Output = tools::Outcome>,
{
    if let Some(reason) =
        crate::chat::hooks::before_tool(config_dir, conversation_id, name, arguments, workdir).await
    {
        return tools::Outcome::err(format!("A hook blocked this call: {reason}"));
    }
    // The agent's vault secrets reach the processes this call starts, and never the model.
    let secrets = crate::chat::vault::env_for(config_dir, agent).await;
    let started = std::time::Instant::now();
    // A written tool records its runs itself, in this run's trace.
    let run = crate::chat::script_tools::in_run(trace.unwrap_or_default().to_string(), run);
    let mut outcome = crate::chat::vault::scope(secrets.clone(), run).await;
    if !secrets.is_empty() {
        outcome.text = crate::chat::vault::mask(&outcome.text, &secrets);
    }
    let shown_args = if secrets.is_empty() {
        arguments.to_string()
    } else {
        crate::chat::vault::mask(&arguments.to_string(), &secrets)
    };
    crate::chat::audit::record(
        config_dir,
        crate::chat::audit::Entry {
            at: Utc::now(),
            kind: "tool".into(),
            conversation: conversation_id.to_string(),
            agent: agent.map(|a| a.name.clone()),
            tool: name.to_string(),
            arguments: crate::chat::audit::short(&shown_args),
            outcome: if outcome.is_error { "error" } else { "ok" }.into(),
            detail: crate::chat::audit::short(&outcome.text),
            millis: started.elapsed().as_millis() as u64,
        },
    );
    if let Some(trace) = trace {
        if name.starts_with("script__") {
            // A written tool records its own runs where it actually ran (also when another tool
            // calls it), so a refusal before it runs (declined, nobody to approve, a changed or
            // unapproved tool, the rate limit) is not counted against it as a failure. A run that
            // fails, a lapsed grant's denial included, is.
            if !outcome.is_error {
                crate::chat::traces::attribute(config_dir, trace, "tool", name, true);
            }
        } else {
            crate::chat::traces::tool_call(
                config_dir,
                trace,
                name,
                started.elapsed().as_millis() as u64,
                !outcome.is_error,
            );
        }
    }
    if !outcome.is_error && crate::chat::traces::empty_result(&outcome.text) {
        crate::chat::traces::signal(config_dir, conversation_id, None, "empty_result");
    }
    let notes = crate::chat::hooks::after_tool(
        config_dir,
        conversation_id,
        name,
        arguments,
        &outcome.text,
        outcome.is_error,
        workdir,
    )
    .await;
    if !notes.is_empty() {
        outcome.text.push_str(&format!(
            "\n\n[From the user's hooks]\n{}",
            notes.join("\n")
        ));
    }
    outcome
}

/// What one tool call needs to run, shared by the calls of a step.
struct Runner<'a> {
    host: &'a Arc<dyn Host>,
    /// The trace this turn records into.
    run: &'a str,
    config_dir: &'a std::path::Path,
    offered: &'a [ChatTool],
    model: &'a DynModel<Completion>,
    request: &'a TurnRequest,
    id: &'a str,
    agent: Option<&'a agents::Agent>,
    /// The chat's folder, where hooks run.
    workdir: Option<String>,
}

impl Runner<'_> {
    /// Runs a call between the user's `before_tool` and `after_tool` hooks.
    async fn run(&self, call: &ToolCall) -> tools::Outcome {
        let arguments = Value::Object(call.function.arguments.clone());
        guarded_call(
            self.config_dir,
            self.id,
            Some(self.run),
            self.agent,
            self.workdir.as_deref().map(std::path::Path::new),
            call.function.name.as_str(),
            &arguments,
            self.run_inner(call),
        )
        .await
    }

    /// A change made outside the connectors (the organization, a scheduled task, a saved skill)
    /// under the approval mode; see [`change_gate`]. `Err` carries the refusal for the model.
    /// `connector` labels the approval card; `None` shows the tool on its own.
    async fn confirm_change(
        &self,
        name: &str,
        connector: Option<String>,
        arguments: &Value,
        asked: bool,
        in_plan: &str,
        unattended: &str,
    ) -> Result<(), tools::Outcome> {
        let refuse = |text: &str| Err(tools::Outcome::err(text));
        match change_gate(
            self.request.approval,
            asked || session_allowed(self.id, name),
        ) {
            ChangeGate::Run => Ok(()),
            ChangeGate::RefuseInPlan => refuse(in_plan),
            ChangeGate::RefuseUnattended => refuse(unattended),
            ChangeGate::Ask => {
                let gate = ChatTool {
                    name: name.into(),
                    description: String::new(),
                    parameters: Value::Null,
                    connector_id: connector,
                    tool_name: name.into(),
                    read_only: false,
                };
                match self.host.approve(&gate, arguments, None).await {
                    ApprovalDecision::Deny => refuse(
                        "The user declined this change. Do not retry it; ask what they would like instead.",
                    ),
                    // "Always allow" lasts for this conversation, as in Vibe.
                    ApprovalDecision::Always => {
                        allow_for_session(self.id, name);
                        Ok(())
                    }
                    ApprovalDecision::Once => Ok(()),
                }
            }
        }
    }

    /// `manage_agents`: a manager agent changes its organization, under the approval mode (it
    /// asks in Ask, is refused in Plan and in scheduled runs nobody watches), then the page's
    /// agent list refreshes.
    async fn manage_agents(&self, arguments: &Value, asked: bool) -> tools::Outcome {
        let refuse = |text: &str| tools::Outcome::err(text);
        let Some(agent) = self.agent else {
            return refuse("Only agents can manage agents.");
        };
        if crate::chat::org::changes(arguments) {
            if let Err(refused) = self
                .confirm_change(
                    crate::chat::org::MANAGE_AGENTS,
                    Some(agent.name.clone()),
                    arguments,
                    asked,
                    "Plan mode changes nothing: describe the agents you would create or change, and the user can switch modes to go ahead.",
                    "Nobody is watching this scheduled run to approve changes to your organization; make them in a chat.",
                )
                .await
            {
                return refused;
            }
        }
        match crate::chat::org::apply(self.config_dir, &agent.id, arguments) {
            Ok((text, changed)) => {
                if changed {
                    if let Some(app) = self.host.app() {
                        let _ = app.emit("chat://agents-changed", ());
                    }
                }
                tools::Outcome {
                    text,
                    is_error: false,
                }
            }
            Err(e) => refuse(&e),
        }
    }

    async fn run_inner(&self, call: &ToolCall) -> tools::Outcome {
        let name = call.function.name.as_str();
        let arguments = Value::Object(call.function.arguments.clone());
        if let Some(raw) = &call.function.invalid_arguments {
            return tools::Outcome::err(format!("The arguments were not a JSON object: {raw}"));
        }
        // Only what this turn offered runs: an agent's allowed tools, a worker's or eval's
        // `only_tools`, the composer's tools turned off, a temporary chat's missing memory tools.
        // A model that names anything else, from an injected page or a guess, is refused.
        if !self.offered.iter().any(|t| t.name == name) {
            return tools::Outcome::err(format!("There is no tool named {name} here."));
        }
        // The agent's "ask first" policy covers every tool, built-in and reading ones included,
        // except those that always ask with their own fuller card (review, scores, access): asking
        // first would replace that card with a plain one.
        let mut asked = false;
        let asks_with_its_own_card = matches!(
            name,
            builtins::SAVE_SKILL
                | crate::chat::script_tools::PROMOTE_TOOL
                | crate::chat::script_tools::REQUEST_CAPABILITY
                | crate::chat::proposals::PROPOSE_CHANGE
        );
        if !asks_with_its_own_card
            && self.agent.is_some_and(|a| agents::asks_first(a, name))
            && !session_allowed(self.id, name)
        {
            if self.request.approval.unattended() {
                return tools::Outcome { text: format!("{name} asks first under this agent's policy, and nobody is watching this scheduled run to approve it."), is_error: true };
            }
            let gate = ChatTool {
                name: name.into(),
                description: String::new(),
                parameters: Value::Null,
                connector_id: Some(
                    self.agent
                        .map_or_else(|| "agent".to_string(), |a| a.name.clone()),
                ),
                tool_name: name.into(),
                read_only: false,
            };
            match self.host.approve(&gate, &arguments, None).await {
                ApprovalDecision::Deny => {
                    return tools::Outcome::err("The user declined this tool call. Do not retry it; ask what they would like instead.");
                }
                // "Always allow" lasts for this conversation, as in Vibe.
                ApprovalDecision::Always => allow_for_session(self.id, name),
                ApprovalDecision::Once => {}
            }
            asked = true;
        }
        if let Some(handler) = handlers::find(name) {
            return handler(self, call, arguments, asked).await;
        }
        match self.offered.iter().find(|t| t.name == name) {
            // Approved just now under the agent's policy: do not ask a second time.
            Some(tool) if asked => {
                tools::execute(
                    self.host.as_ref(),
                    tool,
                    arguments,
                    ApprovalMode::Auto,
                    self.id,
                )
                .await
            }
            Some(tool) => {
                let mode = agents::mode_for(self.agent, name, self.request.approval);
                // Auto: what Ask mode would confirm gets a quick check against the request.
                if mode == ApprovalMode::Auto
                    && tools::risk_here(Some(self.config_dir), tool, &arguments).is_none()
                    && tools::needs_approval(tool, &arguments, ApprovalMode::Ask)
                {
                    if let Some(reason) = self.auto_check(name, &arguments).await {
                        if self.host.approve(tool, &arguments, Some(&reason)).await
                            == ApprovalDecision::Deny
                        {
                            return tools::Outcome::err("The user declined this tool call. Do not retry it; ask what they would like instead.");
                        }
                    }
                }
                tools::execute(self.host.as_ref(), tool, arguments, mode, self.id).await
            }
            None => tools::Outcome::err(format!("There is no tool named {name}.")),
        }
    }
}

impl Runner<'_> {
    /// Background commands in the session's folder; starting one asks like any shell command.
    async fn terminal(&self, name: &str, arguments: &Value) -> tools::Outcome {
        let fail = |text: &str| tools::Outcome::err(text);
        let arg = |k: &str| {
            arguments
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        match name {
            terminal::READ => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => {
                    let chars = arguments
                        .get("chars")
                        .and_then(Value::as_u64)
                        .unwrap_or(4000)
                        .clamp(200, 20_000) as usize;
                    tools::Outcome {
                        text: format!(
                            "Status: {}{}\n\n{}",
                            view.status,
                            view.exit_code
                                .map(|c| format!(" (exit {c})"))
                                .unwrap_or_default(),
                            terminal::tail(&view, chars)
                        ),
                        is_error: false,
                    }
                }
                _ => fail("There is no background command with that id here."),
            },
            terminal::STOP => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => match terminal::stop(&view.id) {
                    Ok(()) => tools::Outcome::ok("Stopped."),
                    Err(e) => fail(&e),
                },
                _ => fail("There is no background command with that id here."),
            },
            _ => {
                let Some(dir) = store::load_conversation(self.config_dir, self.id)
                    .ok()
                    .and_then(|c| c.workdir)
                else {
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
                // Smart-approve: a command that is hard to undo, or that names the app's own
                // data, asks even in Full auto.
                let risky = tools::risk_here(Some(self.config_dir), &gate, arguments);
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
                            if self.host.approve(&gate, arguments, Some(&reason)).await == ApprovalDecision::Deny {
                                return fail("The user declined to start this command. Do not retry it; ask what they would like instead.");
                            }
                        }
                    }
                    _ => {
                        if self.host.approve(&gate, arguments, None).await == ApprovalDecision::Deny {
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
                                view.as_ref()
                                    .map(|v| terminal::tail(v, 2000))
                                    .unwrap_or_default()
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
            .map(crate::chat::context::message_text)
            .unwrap_or_default();
        let prompt = tools::auto_check_prompt(&request, tool, arguments);
        let call = rig_core::completion::CompletionRequest::new(Message::user(prompt));
        match self.model.call(call).await {
            Ok(response) => tools::auto_check_verdict(&response.text()),
            // No answer: asking is the safe side.
            Err(e) => Some(format!(
                "the safety check could not run ({})",
                crate::chat::context::short_error(&e.to_string())
            )),
        }
    }

    /// Runs code once the user allows it: every run in "ask" modes, unless they allowed all, and
    /// every run in every mode where it would not be isolated.
    async fn run_code(&self, arguments: &Value) -> tools::Outcome {
        let gate = sandbox::code_gate(
            self.request.approval,
            sandbox::isolated(self.config_dir),
            sandbox::always_allowed(self.config_dir),
        );
        match gate {
            sandbox::CodeGate::Refuse(text) => return tools::Outcome::err(text),
            sandbox::CodeGate::Run => {}
            sandbox::CodeGate::Ask(reason) => {
                let Some(tool) = self.offered.iter().find(|t| t.name == sandbox::RUN_CODE) else {
                    return tools::Outcome::err("Running code is not available.");
                };
                let gate = ChatTool {
                    connector_id: Some("sandbox".into()),
                    ..tool.clone()
                };
                match self.host.approve(&gate, arguments, reason).await {
                    ApprovalDecision::Deny => {
                        return tools::Outcome::err("The user declined to run this code. Do not retry it; ask what they would like instead.")
                    }
                    // "Always" covers isolated runs; an unsandboxed one asks every time.
                    ApprovalDecision::Always if reason.is_none() => {
                        if let Err(e) = sandbox::set_always_allowed(self.config_dir, true) {
                            warn!("could not save the code-run permission: {e}");
                        }
                    }
                    ApprovalDecision::Always | ApprovalDecision::Once => {}
                }
            }
        }
        // An agent's code runs in its own environment (home, packages), kept across its chats.
        let env = self
            .agent
            .and_then(|a| sandbox::agent_env(self.config_dir, &a.id, a.code_network).ok());
        sandbox::run_in(self.config_dir, self.id, arguments, env.as_ref()).await
    }
}

/// Puts the model's question to the user and waits for the answer.
async fn ask_user(
    host: &dyn Host,
    id: &str,
    call: &ToolCall,
    cancel: &mut watch::Receiver<bool>,
) -> tools::Outcome {
    let arguments = Value::Object(call.function.arguments.clone());
    let call_id = call.id.to_string();
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let options = arguments
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let state = host.chat_state();
    let answer = state.ask(&call_id);
    host.emit(ChatEvent::Question {
        conversation_id: id.into(),
        call_id: call_id.clone(),
        question,
        options,
    });
    tokio::select! {
        _ = cancelled(cancel) => {
            state.forget_question(&call_id);
            tools::Outcome::err("The user dismissed the question.")
        }
        answer = answer => match answer {
            Ok(text) => tools::Outcome { text, is_error: false },
            Err(_) => tools::Outcome::err("The user dismissed the question."),
        }
    }
}

/// Summarizes the turns before the last few once the conversation nears the model's window.
/// What [`compact_if_needed`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compaction {
    /// The context fits, or there is nothing older than the recent turns to summarize.
    NotNeeded,
    Done,
    Failed,
}

async fn compact_if_needed(
    host: &dyn Host,
    model: &DynModel<Completion>,
    conversation: &mut Conversation,
    window: u64,
    config_dir: &std::path::Path,
) -> Compaction {
    let measured = if conversation.usage.context_tokens > 0 {
        conversation.usage.context_tokens
    } else {
        context::estimate_tokens(&context::request_messages(
            &conversation.messages,
            conversation.summary.as_ref(),
        ))
    };
    if !context::should_compact(measured, window) {
        return Compaction::NotNeeded;
    }
    let covered = conversation.summary.as_ref().map_or(0, |s| s.upto);
    let Some(point) = context::compaction_point(&conversation.messages, covered) else {
        return Compaction::NotNeeded;
    };
    let id = conversation.id.clone();
    host.emit(ChatEvent::Compacting {
        conversation_id: id.clone(),
    });
    let transcript = context::transcript(&conversation.messages[covered..point]);
    let prompt = context::summary_prompt(
        conversation.summary.as_ref().map(|s| s.text.as_str()),
        &transcript,
    );
    let request = CompletionRequest::new(Message::user(prompt)).max_tokens(1500u64);
    match tokio::time::timeout(std::time::Duration::from_secs(120), model.call(request)).await {
        Ok(Ok(response)) if !response.text().trim().is_empty() => {
            info!("compacted conversation {id} up to message {point}");
            conversation.usage.add(&response.usage);
            conversation.summary = Some(context::Summary {
                text: response.text().trim().to_string(),
                upto: point,
                created_at: Utc::now(),
            });
            // The next reply measures the smaller context afresh.
            conversation.usage.context_tokens = 0;
            if let Err(e) = store::save_conversation(config_dir, conversation) {
                warn!("could not save compacted conversation: {e}");
            }
            host.emit(ChatEvent::Compacted {
                conversation_id: id,
                upto: point,
            });
            Compaction::Done
        }
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
            warn!("compaction failed; continuing with tool-result clearing only");
            host.emit(ChatEvent::Compacted {
                conversation_id: id,
                upto: covered,
            });
            Compaction::Failed
        }
    }
}

/// Runs a scheduled task: a new unread conversation with the task's prompt, under its agent.
pub async fn run_task(app: AppHandle, task: tasks::Task) {
    let Ok(config_dir) = config_dir(&app) else {
        return;
    };
    let agent = task
        .agent_id
        .as_deref()
        .and_then(|a| agents::find(&config_dir, a));
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
    // A follow-up continues the chat it was asked in, while that chat still exists.
    let follow_up = task
        .reply_in
        .as_ref()
        .and_then(|id| store::load_conversation(&config_dir, id).ok())
        .filter(|c| !c.archived);
    let id = match follow_up {
        Some(mut conversation) => {
            conversation.unread = true;
            if let Err(e) = store::save_conversation(&config_dir, &conversation) {
                warn!("could not continue a chat for {}: {e}", task.title);
                return;
            }
            conversation.id
        }
        None => {
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
            id
        }
    };
    let prompt = tasks::run_prompt(&task);
    let text = if task.reply_in.as_deref() == Some(id.as_str()) {
        format!("(Scheduled follow-up: {}) {}", task.title, prompt)
    } else {
        prompt
    };
    let Some(cancel) = app.state::<ChatState>().begin(&id) else {
        return;
    };
    let request = TurnRequest {
        conversation_id: id.clone(),
        text,
        model,
        speed: if agent.as_ref().is_some_and(|a| a.think) {
            Speed::Think
        } else {
            Speed::Fast
        },
        approval: if task.allow_changes {
            ApprovalMode::Autonomous
        } else {
            ApprovalMode::Unattended
        },
        project_id: None,
        agent_id: task.agent_id.clone(),
        images: Vec::new(),
        edit_index: None,
        temporary: false,
        resume: false,
        workdir: None,
        connectors: None,
        route_note: None,
        tools_off: Vec::new(),
        max_steps: None,
        only_tools: None,
        system: None,
        skills_off: Vec::new(),
        facts_off: Vec::new(),
        pages_off: Vec::new(),
        instructions_off: false,
    };
    run_turn(app.clone(), request, cancel).await;
    let finished = store::load_conversation(&config_dir, &id).ok();
    let answered = finished
        .as_ref()
        .and_then(|c| c.messages.last())
        .is_some_and(|m| matches!(m, Message::Assistant(_)));
    tasks::record(
        &config_dir,
        &task.id,
        &id,
        if answered { "ok" } else { "failed" },
    );
    let answer = finished
        .as_ref()
        .and_then(|c| c.messages.last())
        .map(crate::chat::context::message_text)
        .unwrap_or_default();
    tasks::notify_webhook(&task, &id, if answered { "ok" } else { "failed" }, &answer).await;
    notify(
        &app,
        &task.title,
        if answered {
            "Your scheduled task finished."
        } else {
            "Your scheduled task could not finish."
        },
    );
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
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
        } else if let Some(r) = rest
            .strip_prefix("Use the \"")
            .and_then(|r| r.split_once("\" skill."))
            .map(|(_, r)| r)
        {
            rest = r.trim_start();
        } else if rest.starts_with("Create ")
            && rest.contains(" with generate_image (")
            && rest.contains("\n\n")
        {
            rest = rest.split_once("\n\n").map_or("", |(_, r)| r).trim_start();
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
    let line = strip_directives(&first)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if line.chars().count() > 48 {
        format!("{}…", line.chars().take(47).collect::<String>().trim_end())
    } else if line.is_empty() {
        "New chat".into()
    } else {
        line
    }
}

/// Asks the same model for a short title, falling back to the first message.
async fn name_conversation(host: Arc<dyn Host>, reference: String, mut conversation: Conversation) {
    let Ok(config_dir) = host.config_dir() else {
        return;
    };
    let mut title = fallback_title(&conversation);
    let prompt = format!(
        "Write a title of at most 6 words for a conversation that starts with the message \
         below, in the message's language. Reply with the title only, no quotes.\n\n{}",
        title
    );
    // Room for models that think before they answer; the title itself is a few words.
    let request = CompletionRequest::new(Message::user(prompt)).max_tokens(600u64);
    if let Ok(text) = ask_text(&config_dir, &reference, request, 20).await {
        let text = text.trim().trim_matches(['"', '\'', '*', '#']).trim();
        if !text.is_empty() && text.chars().count() <= 80 {
            title = text.to_string();
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
            host.emit(ChatEvent::Title {
                conversation_id: latest.id,
                title,
            });
        }
    }
}

/// Replaces screenshots in earlier tool results with a note: only the latest view matters, and
/// each one costs many tokens.
fn drop_old_screenshots(messages: &mut [Message]) {
    use rig_core::completion::message::UserContent;
    for message in messages.iter_mut() {
        let Message::User { content } = message else {
            continue;
        };
        for item in content.iter_mut() {
            if let UserContent::ToolResult(result) = item {
                let has_image = result
                    .content
                    .iter()
                    .any(|c| matches!(c, ToolResultContent::Image(_)));
                if has_image {
                    let kept: Vec<ToolResultContent> = result
                        .content
                        .iter()
                        .filter(|c| !matches!(c, ToolResultContent::Image(_)))
                        .cloned()
                        .chain(std::iter::once(ToolResultContent::text(
                            "[earlier screenshot removed]",
                        )))
                        .collect();
                    result.content = kept;
                }
            }
        }
    }
}

mod handlers;

#[cfg(test)]
mod turn_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_outside_the_connectors_follow_the_approval_mode() {
        use ApprovalMode::*;
        assert_eq!(change_gate(Ask, false), ChangeGate::Ask);
        assert_eq!(change_gate(AcceptEdits, false), ChangeGate::Ask);
        // Approved just now under an agent's policy, or "Always allow" earlier in the chat.
        assert_eq!(change_gate(Ask, true), ChangeGate::Run);
        assert_eq!(change_gate(Auto, false), ChangeGate::Run);
        assert_eq!(change_gate(Autonomous, false), ChangeGate::Run);
        // An approval never lets a change through where changes are refused.
        assert_eq!(change_gate(Plan, true), ChangeGate::RefuseInPlan);
        assert_eq!(change_gate(Unattended, true), ChangeGate::RefuseUnattended);
    }

    #[test]
    fn the_prompt_uses_the_users_local_time() {
        use chrono::TimeZone as _;
        let lisbon_summer = chrono::FixedOffset::east_opt(3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 1, 0, 30, 0)
            .unwrap();
        let line = local_time_line(lisbon_summer);
        // 00:30 at UTC+1 is July 1 for the user but still June 30 in UTC.
        assert!(
            line.starts_with("Today is Wednesday, July 1, 2026 in the user's time zone ("),
            "{line}"
        );
        assert!(line.contains("UTC+01:00"), "{line}");
    }

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
            &Context {
                instructions: Some(&instructions),
                project: Some(&project),
                workdir: Some("/src/app"),
                ..Default::default()
            },
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
        let png = ImageInput {
            media_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };
        let m = user_message("look", std::slice::from_ref(&png)).unwrap();
        let Message::User { content } = m else {
            panic!()
        };
        assert_eq!(content.len(), 2);
        assert!(user_message(
            "x",
            &[ImageInput {
                media_type: "image/tiff".into(),
                data: "AA==".into()
            }]
        )
        .is_err());
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
        assert_eq!(
            strip_directives(
                "Use a canvas for this.\n\nUse the \"deep-research\" skill.\n\nAI in 2026?"
            ),
            "AI in 2026?"
        );
        assert_eq!(
            strip_directives("> quoted line\n> more\n\nWhat does it mean?"),
            "What does it mean?"
        );
        assert_eq!(strip_directives("Plain question"), "Plain question");
        assert_eq!(strip_directives("Create 2 images (variations) with generate_image (shape: square, count: 2). This is a mascot.\n\nmascot for ERSE"), "mascot for ERSE");
    }

    #[test]
    fn titles_fall_back_to_the_first_line() {
        let mut c = Conversation::new("c".into(), None, None);
        assert_eq!(fallback_title(&c), "New chat");
        c.messages
            .push(Message::user("Plan a trip to Lisbon\nwith kids"));
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
