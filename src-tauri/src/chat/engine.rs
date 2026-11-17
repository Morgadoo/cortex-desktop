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

use super::{builtins, canvas, context, memory, providers, skills, subagents};
use super::store::{self, Conversation, Instructions, Project};
use super::tools::{self, ApprovalMode, ChatTool, ASK_USER};
use super::{ChatState, EVENT, LOCAL_ACCOUNT};
use crate::account::AccountId;
use crate::tools::auth::keychain_secrets;
use crate::tools::registry::ToolRegistry;

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
    #[serde(rename_all = "camelCase")]
    Error { conversation_id: String, message: String },
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
}

pub fn preamble(context: &Context<'_>, tools: &[ChatTool]) -> String {
    let default_instructions = Instructions::default();
    let instructions = context.instructions.unwrap_or(&default_instructions);
    let project = context.project;
    let workdir = context.workdir;
    let mut out = String::from(
        "You are a helpful, accurate assistant running in a desktop app. Answer in the user's \
         language. Use Markdown: short paragraphs, lists and tables where they help, fenced code \
         blocks with a language tag. When you are unsure, say so instead of guessing.",
    );
    out.push_str(&format!("\n\nToday is {}.", Utc::now().format("%A, %B %-d, %Y")));
    if !instructions.name.trim().is_empty() {
        out.push_str(&format!(" The user's name is {}.", instructions.name.trim()));
    }
    if let Some((_, tone)) = TONES.iter().find(|(id, _)| *id == instructions.tone) {
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
    }
    if let Some(dir) = workdir {
        out.push_str(&format!(
            "\n\nThis is a coding session in the folder `{dir}`. Read the code before changing \
             it, keep changes focused, run the project's own build and test commands to check \
             your work, and report what you changed."
        ));
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
    out
}

/// The API key for `vendor` from the keychain, if one is saved.
async fn api_key(vendor: &str) -> Result<Option<String>, String> {
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
    let last_prompt = conversation.messages.iter().rposition(|m| match m {
        Message::User { content } => content
            .iter()
            .any(|c| matches!(c, rig_core::completion::message::UserContent::Text(_))),
        _ => false,
    });
    match last_prompt {
        Some(index) => {
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
            c
        }
    };
    conversation.model = Some(request.model.clone());

    if request.resume {
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
        conversation.messages.push(Message::user(request.text.trim()));
        conversation.times.push((conversation.messages.len() - 1, Utc::now()));
    }
    conversation.updated_at = Utc::now();
    store::save_conversation(&config_dir, &conversation)?;

    let model = connect_model(&config_dir, &request.model).await?;
    let registry = app.state::<ToolRegistry>();
    let mut offered = tools::available(&registry, request.connectors.as_deref());
    offered.splice(1..1, builtins::definitions(&config_dir));
    let definitions: Vec<_> = offered.iter().filter_map(ChatTool::definition).collect();
    let project = conversation
        .project_id
        .as_ref()
        .and_then(|pid| store::load_projects(&config_dir).into_iter().find(|p| &p.id == pid));
    let instructions = store::load_instructions(&config_dir);
    let memories = memory::load(&config_dir);
    let skill_list = skills::list(&config_dir);
    let canvases = canvas::list_for(&config_dir, id);
    let system = preamble(
        &Context {
            instructions: Some(&instructions),
            project: project.as_ref(),
            workdir: conversation.workdir.as_deref(),
            memories: Some(&memories),
            skills: &skill_list,
            canvases: &canvases,
        },
        &offered,
    );
    let reasoning = providers::supports_reasoning(&request.model);
    let window = context::context_window(&request.model);
    compact_if_needed(app, &model, &mut conversation, window, &config_dir).await;

    for step in 0..MAX_STEPS {
        let carried = context::request_messages(&conversation.messages, conversation.summary.as_ref());
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
        if last_step {
            step_system.push_str("\n\nYou have used every tool step available for this request. Answer now from what you have found, and say what you could not finish.");
        }
        let mut call = CompletionRequest::new(prompt.clone())
            .messages(history.iter().cloned())
            .preamble(step_system);
        if !last_step {
            call = call.tools(definitions.iter().cloned());
        }
        if reasoning && request.speed == Speed::Think {
            call = call.reasoning(Effort::High);
        }

        // Transient failures (dropped connections, rate limits, overloaded servers) are retried
        // with backoff, as long as nothing reached the user yet.
        let mut attempt = 0u32;
        let response = loop {
            attempt += 1;
            let mut stream = model.stream(call.clone()).map_err(|e| e.to_string())?;
            let mut streamed_text = String::new();
            let mut streamed_anything = false;
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
                                streamed_text.push_str(&text);
                                emit(app, ChatEvent::Text { conversation_id: id.into(), text });
                            }
                            StreamEvent::Reasoning { text, .. } => {
                                streamed_anything = true;
                                emit(app, ChatEvent::Reasoning { conversation_id: id.into(), text });
                            }
                            StreamEvent::Start { kind: PartKind::ToolCall, name: Some(name), .. } => {
                                streamed_anything = true;
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
                Err(e) => {
                    store::save_conversation(&config_dir, &conversation)?;
                    return Err(e.to_string());
                }
            }
        };
        conversation.usage.add(&response.usage);
        emit(app, ChatEvent::Usage { conversation_id: id.into(), usage: conversation.usage.clone(), context_window: window });
        let calls: Vec<ToolCall> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            conversation.messages.push(message);
        }
        conversation.updated_at = Utc::now();

        if calls.is_empty() {
            conversation.times.push((conversation.messages.len() - 1, Utc::now()));
            store::save_conversation(&config_dir, &conversation)?;
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
                || offered.iter().find(|t| t.name == name).is_some_and(|tool| {
                    tool.connector_id.is_some()
                        && tools::needs_approval(tool, &Value::Object(call.function.arguments.clone()), request.approval)
                })
        };
        let mut outcomes: Vec<Option<tools::Outcome>> = (0..calls.len()).map(|_| None).collect();
        let runner = Runner { app, config_dir: &config_dir, registry: &registry, offered: &offered, model: &model, request, id };
        let concurrent = futures::future::join_all(
            calls.iter().enumerate().filter(|(_, c)| !waits_on_user(c)).map(|(i, c)| {
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
            if outcomes[i].is_some() || !waits_on_user(call) || *cancel.borrow() {
                continue;
            }
            let outcome = if call.function.name.as_str() == ASK_USER {
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
            let mut result = call.result(vec![ToolResultContent::text(outcome.text)]);
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

fn cancelled_outcome() -> tools::Outcome {
    tools::Outcome { text: "Cancelled by the user.".into(), is_error: true }
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
}

impl Runner<'_> {
    async fn run(&self, call: &ToolCall) -> tools::Outcome {
        let name = call.function.name.as_str();
        let arguments = Value::Object(call.function.arguments.clone());
        if let Some(raw) = &call.function.invalid_arguments {
            return tools::Outcome { text: format!("The arguments were not a JSON object: {raw}"), is_error: true };
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
        if name == builtins::CANVAS {
            let text = |k: &str| arguments.get(k).and_then(Value::as_str).map(str::to_string);
            let Some(content) = text("content").filter(|c| !c.trim().is_empty()) else {
                return tools::Outcome { text: "Give the canvas content.".into(), is_error: true };
            };
            let id = text("id").filter(|i| !i.trim().is_empty());
            return match canvas::write(self.config_dir, self.id, id.as_deref(), text("title").as_deref(), &content, "assistant") {
                Ok(c) => tools::Outcome {
                    text: serde_json::json!({ "id": c.id, "title": c.title, "version": c.versions.len() }).to_string(),
                    is_error: false,
                },
                Err(e) => tools::Outcome { text: e, is_error: true },
            };
        }
        if builtins::is_builtin(name) {
            return builtins::run(self.config_dir, name, &arguments).await;
        }
        match self.offered.iter().find(|t| t.name == name) {
            Some(tool) => tools::execute(self.app, self.registry, tool, arguments, self.request.approval, self.id).await,
            None => tools::Outcome { text: format!("There is no tool named {name}."), is_error: true },
        }
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
        let project = Project { id: "p".into(), name: "Garden".into(), instructions: "Plants only.".into() };
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
