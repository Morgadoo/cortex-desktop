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

use super::{builtins, memory, providers, skills};
use super::store::{self, Conversation, Instructions, Project};
use super::tools::{self, ApprovalMode, ChatTool, ASK_USER};
use super::{ChatState, EVENT, LOCAL_ACCOUNT};
use crate::account::AccountId;
use crate::tools::auth::keychain_secrets;
use crate::tools::registry::ToolRegistry;

/// Model calls in one turn before the engine stops a tool loop that does not converge. The last
/// step withholds the tools, so a long research run still ends in an answer.
const MAX_STEPS: usize = 24;

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
    /// Code sessions: the folder to work in. Only read when the conversation is created.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Connector ids to offer; all enabled ones when absent.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
}

fn emit(app: &AppHandle, event: ChatEvent) {
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
    if let Some(memories) = context.memories.and_then(memory::prompt) {
        out.push_str(&memories);
    }
    if let Some(skills) = skills::prompt(context.skills) {
        out.push_str(&skills);
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

    if request.text.trim().is_empty() {
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
    let system = preamble(
        &Context {
            instructions: Some(&instructions),
            project: project.as_ref(),
            workdir: conversation.workdir.as_deref(),
            memories: Some(&memories),
            skills: &skill_list,
        },
        &offered,
    );
    let reasoning = providers::supports_reasoning(&request.model);

    for step in 0..MAX_STEPS {
        let Some((prompt, history)) = conversation.messages.split_last() else {
            return Err("empty conversation".into());
        };
        let last_step = step + 1 == MAX_STEPS;
        let mut call = CompletionRequest::new(prompt.clone())
            .messages(history.iter().cloned())
            .preamble(if last_step {
                format!("{system}\n\nYou have used every tool step available for this request. Answer now from what you have found, and say what you could not finish.")
            } else {
                system.clone()
            });
        if !last_step {
            call = call.tools(definitions.iter().cloned());
        }
        if reasoning && request.speed == Speed::Think {
            call = call.reasoning(Effort::High);
        }

        let mut stream = model.stream(call).map_err(|e| e.to_string())?;
        let mut streamed_text = String::new();
        let was_cancelled = loop {
            tokio::select! {
                _ = cancelled(&mut cancel) => break true,
                item = stream.next() => match item {
                    None => break false,
                    Some(Err(_)) => break false, // finish() reports it
                    Some(Ok(Item::Unknown(_))) => {}
                    Some(Ok(Item::Event(event))) => match event {
                        StreamEvent::Text { text, .. } => {
                            streamed_text.push_str(&text);
                            emit(app, ChatEvent::Text { conversation_id: id.into(), text });
                        }
                        StreamEvent::Reasoning { text, .. } => {
                            emit(app, ChatEvent::Reasoning { conversation_id: id.into(), text });
                        }
                        StreamEvent::Start { kind: PartKind::ToolCall, name: Some(name), .. } => {
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

        let response = match stream.finish().await {
            Ok(response) => response,
            Err(e) => {
                store::save_conversation(&config_dir, &conversation)?;
                return Err(e.to_string());
            }
        };
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

        let mut results = Vec::with_capacity(calls.len());
        for call in &calls {
            let call_id = call.id.to_string();
            let name = call.function.name.to_string();
            let arguments = Value::Object(call.function.arguments.clone());
            emit(app, ChatEvent::ToolStart {
                conversation_id: id.into(),
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            });

            let outcome = if let Some(raw) = &call.function.invalid_arguments {
                tools::Outcome { text: format!("The arguments were not a JSON object: {raw}"), is_error: true }
            } else if name == ASK_USER {
                let question = arguments.get("question").and_then(Value::as_str).unwrap_or_default().to_string();
                let options = arguments
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default();
                let state = app.state::<ChatState>();
                let answer = state.ask(&call_id);
                emit(app, ChatEvent::Question {
                    conversation_id: id.into(),
                    call_id: call_id.clone(),
                    question,
                    options,
                });
                tokio::select! {
                    _ = cancelled(&mut cancel) => {
                        state.forget_question(&call_id);
                        tools::Outcome { text: "The user dismissed the question.".into(), is_error: true }
                    }
                    answer = answer => match answer {
                        Ok(text) => tools::Outcome { text, is_error: false },
                        Err(_) => tools::Outcome { text: "The user dismissed the question.".into(), is_error: true },
                    }
                }
            } else if builtins::is_builtin(&name) {
                tokio::select! {
                    _ = cancelled(&mut cancel) => tools::Outcome { text: "Cancelled by the user.".into(), is_error: true },
                    outcome = builtins::run(&config_dir, &name, &arguments) => outcome,
                }
            } else if let Some(tool) = offered.iter().find(|t| t.name == name) {
                tokio::select! {
                    _ = cancelled(&mut cancel) => tools::Outcome { text: "Cancelled by the user.".into(), is_error: true },
                    outcome = tools::execute(app, &registry, tool, arguments, request.approval, id) => outcome,
                }
            } else {
                tools::Outcome { text: format!("There is no tool named {name}."), is_error: true }
            };

            emit(app, ChatEvent::ToolEnd {
                conversation_id: id.into(),
                call_id,
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
    let line = first.lines().next().unwrap_or_default().trim().to_string();
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
