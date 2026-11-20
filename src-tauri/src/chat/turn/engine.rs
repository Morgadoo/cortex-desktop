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
use crate::chat::ChatState;
use crate::chat::{
    agents, builtins, canvas, context, deferred, knowledge, libraries, media, memory, privacy,
    providers, sandbox, skills, subagents, tasks, terminal,
};
use crate::tools::registry::ApprovalDecision;

/// Model calls in one turn before the engine stops a tool loop that does not converge. The last
/// step withholds the tools, so a long research run still ends in an answer.
const MAX_STEPS: usize = 24;
/// Attempts per model call when the failure is transient.
const MAX_ATTEMPTS: u32 = 3;

pub use crate::chat::connect::{ask_text, connect_model};
pub(in crate::chat) use crate::chat::events::emit;
pub use crate::chat::events::ChatEvent;

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

    apply_request(&mut conversation, request, &text)?;
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

    // Some "models" are not models: a pipe (a function of the user's), an ACP agent, or a coding
    // agent program (Claude Code, Codex, Gemini CLI) answers in their place, with its own tools.
    if let Some(pipe) = crate::chat::functions::find_pipe(&config_dir, &request.model) {
        return answer_with_pipe(host, request, &config_dir, conversation, &mut cancel, &pipe)
            .await;
    }
    if let Some(acp_agent) = crate::chat::acp::find(&config_dir, &request.model) {
        let named = !conversation.title.is_empty();
        let app = host.app().ok_or(NO_APP)?;
        let cancelled =
            crate::chat::acp::run(app, &config_dir, conversation.clone(), &acp_agent, cancel)
                .await?;
        name_unless(named, host, request, conversation);
        return Ok(cancelled);
    }
    if let Some(cli) = crate::chat::agent_cli::find(&request.model) {
        let named = !conversation.title.is_empty();
        let app = host.app().ok_or(NO_APP)?;
        let cancelled =
            crate::chat::agent_cli::run(app, &config_dir, conversation.clone(), cli, cancel)
                .await?;
        name_unless(named, host, request, conversation);
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
    let (mut offered, defer) = tools_for_turn(
        registry,
        &config_dir,
        request,
        &conversation,
        agent.as_ref(),
    );
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
    add_turn_sections(
        &mut prompt,
        &config_dir,
        request,
        &conversation,
        agent.as_ref(),
        &offered,
        defer,
        &memories,
        &pages,
        &latest_prompt,
    );
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
    name_unless(!conversation.title.is_empty(), host, request, conversation);
    Ok(false)
}

fn cancelled_outcome() -> tools::Outcome {
    tools::Outcome::err("Cancelled by the user.")
}

use cortex_kernel::approval::{change_gate, ChangeGate};

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

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        warn!("could not show a notification: {e}");
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

mod compaction;
mod handlers;
mod prompt;
mod reflection;
mod request;
mod routing;
mod runner;
mod scheduled;
mod steps;
mod titles;
mod tool_output;
use compaction::*;
pub use prompt::*;
use reflection::*;
pub use request::*;
use routing::*;
use runner::*;
pub use scheduled::*;
use steps::*;
pub use titles::*;
pub use tool_output::*;

mod turn_parts;
use turn_parts::*;

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
