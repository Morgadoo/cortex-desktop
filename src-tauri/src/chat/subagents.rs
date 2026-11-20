//! Sub-agents: independent workers the main assistant hands parts of a task to.
//!
//! The orchestrator-worker pattern, for context isolation and parallelism. Each worker starts
//! from a clean context holding only its task, uses read-only tools in a short loop of its own,
//! and returns a condensed report, so the main conversation receives a few hundred words per
//! task instead of every page a worker read. Workers run concurrently, can't ask the user
//! anything, can't change anything, and can't delegate further.

use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::future::join_all;
use futures::FutureExt;
use rig_core::completion::Message;
use rig_core::operation::Completion;
use rig_core::DynModel;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::engine::{ChatEvent, TurnRequest};
use crate::chat::host::Host;
use crate::chat::tools::{ApprovalMode, ChatTool};
use crate::chat::{builtins, context, store};

/// Tool calls a worker may make before it must report.
pub const MAX_WORKER_STEPS: usize = 8;
/// Workers per `delegate` call.
pub const MAX_TASKS: usize = 5;
/// Characters of a worker's report passed back to the main conversation.
pub const MAX_REPORT_CHARS: usize = 6_000;

/// Tools a worker gets: reading and searching only.
const WORKER_TOOLS: &[&str] = &[
    builtins::WEB_SEARCH,
    builtins::USE_SKILL,
    "browser__navigate",
    "browser__fetch",
    "filesystem__fs_search",
    "filesystem__fs_read",
];

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub title: String,
    pub instructions: String,
    /// The agent to hand the task to, by name; a generic worker when absent.
    #[serde(default)]
    pub agent: Option<String>,
    /// The tools this task needs, by name, from those workers may use; all of them when empty.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// The tools one task's worker gets: those workers may use, that its agent allows, and, when the
/// task names some, only those (the kernel's `spawn(role, task, tools)`).
pub fn task_tools(
    worker_tools: &[ChatTool],
    agent: Option<&crate::chat::agents::Agent>,
    wanted: &[String],
) -> Vec<ChatTool> {
    worker_tools
        .iter()
        .filter(|t| agent.is_none_or(|a| crate::chat::agents::allows(a, &t.name)))
        .filter(|t| wanted.is_empty() || wanted.contains(&t.name))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    pub title: String,
    /// `done` or `failed`.
    pub status: &'static str,
    pub report: String,
    pub steps: usize,
}

pub fn worker_tools(offered: &[ChatTool]) -> Vec<ChatTool> {
    offered
        .iter()
        .filter(|t| WORKER_TOOLS.contains(&t.name.as_str()))
        .cloned()
        .collect()
}

/// How a `delegate` call's tasks run (LobeHub's agent group modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// All at once, each on its own.
    Parallel,
    /// One after another, each seeing what the earlier ones reported.
    Sequential,
    /// All at once, then a second round where each answers the others' reports.
    Debate,
}

impl Mode {
    pub fn parse(args: &Value) -> Mode {
        match args.get("mode").and_then(Value::as_str).unwrap_or_default() {
            "sequential" => Mode::Sequential,
            "debate" => Mode::Debate,
            _ => Mode::Parallel,
        }
    }
}

/// The task as the next member sees it, with what the earlier ones reported.
pub fn after(task: &Task, earlier: &[Report]) -> Task {
    if earlier.is_empty() {
        return task.clone();
    }
    let notes: Vec<String> = earlier
        .iter()
        .map(|r| format!("### {}\n{}", r.title, r.report.trim()))
        .collect();
    Task {
        instructions: format!(
            "{}\n\nWhat the members before you reported (build on it; do not repeat it):\n\n{}",
            task.instructions,
            notes.join("\n\n")
        ),
        ..task.clone()
    }
}

/// A member's second-round task in a debate: the others' first answers, to answer.
pub fn rebuttal(task: &Task, own: usize, first: &[Report]) -> Task {
    let others: Vec<String> = first
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != own)
        .map(|(_, r)| format!("### {}\n{}", r.title, r.report.trim()))
        .collect();
    Task {
        title: format!("Round 2 · {}", task.title),
        instructions: format!(
            "{}\n\nYou answered this already. Here are the other members' answers:\n\n{}\n\nReply with where you agree, where you disagree and why, and your revised answer. Keep what you still hold; change what they convinced you of.",
            task.instructions,
            others.join("\n\n")
        ),
        agent: task.agent.clone(),
        tools: task.tools.clone(),
    }
}

pub fn parse_tasks(args: &Value) -> Result<Vec<Task>, String> {
    let tasks: Vec<Task> =
        serde_json::from_value(args.get("tasks").cloned().unwrap_or(Value::Null))
            .map_err(|_| "`tasks` must be a list of {title, instructions}.".to_string())?;
    if tasks.is_empty() {
        return Err("Give at least one task.".into());
    }
    if tasks.len() > MAX_TASKS {
        return Err(format!("At most {MAX_TASKS} tasks at once."));
    }
    Ok(tasks)
}

fn worker_preamble(task: &Task, agent: Option<&crate::chat::agents::Agent>) -> String {
    let persona = agent.map_or(String::new(), |a| {
        format!(
            "You are {}, an agent with this role: {}\n\n{}{}\n\n",
            a.label(),
            a.description.trim(),
            a.role_prompt(),
            a.instructions.trim()
        )
    });
    persona
        + &format!(
        "You are a research worker helping another assistant. Complete exactly this task, then \
         stop:\n\n## {}\n{}\n\nUse the tools to find what the task needs; prefer primary and \
         official sources and open the most promising results. You cannot ask questions, so \
         make reasonable assumptions and say which. Finish with a report of at most 300 words: \
         the findings first, then key facts and figures, then the URLs you relied on. Today is {}.",
        task.title,
        task.instructions,
        chrono::Utc::now().format("%B %-d, %Y")
    )
}

/// Whether a worker's last message is a report, not nothing or a tool call written as text.
fn is_report(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.starts_with("[called tool")
        && !text.starts_with("<tool_call")
        && text.split_whitespace().count() >= 8
}

fn short(text: &str, max: usize) -> String {
    let text: String = text.chars().take(max).collect();
    text.trim().to_string()
}

/// One line saying what a worker is doing, from a tool call's name and arguments.
fn activity_of(name: &str, args: &Value) -> String {
    let detail = ["query", "url", "path"]
        .iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
        .unwrap_or_default();
    let what = match name {
        builtins::WEB_SEARCH => "Searching",
        "browser__navigate" | "browser__fetch" => "Reading",
        "filesystem__fs_search" => "Searching files",
        "filesystem__fs_read" => "Reading file",
        builtins::USE_SKILL => "Loading skill",
        other => other,
    };
    format!("{what} {}", short(detail, 80)).trim().to_string()
}

/// A worker's turn, as the engine runs it: progress goes to the parent chat's sub-agent row, the
/// tool steps to the transcript, and everything else (data, models, approvals) is the parent's.
struct WorkerHost {
    parent: Arc<dyn Host>,
    state: crate::chat::ChatState,
    conversation_id: String,
    call_id: String,
    index: usize,
    title: String,
    steps: Mutex<Vec<Step>>,
}

impl WorkerHost {
    fn progress(&self, status: &str, activity: String) {
        self.parent.emit(ChatEvent::Subagent {
            conversation_id: self.conversation_id.clone(),
            call_id: self.call_id.clone(),
            index: self.index,
            title: self.title.clone(),
            status: status.into(),
            activity,
        });
    }

    fn steps(&self) -> std::sync::MutexGuard<'_, Vec<Step>> {
        self.steps.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait::async_trait]
impl Host for WorkerHost {
    fn config_dir(&self) -> Result<std::path::PathBuf, String> {
        self.parent.config_dir()
    }

    fn emit(&self, event: ChatEvent) {
        match event {
            ChatEvent::ToolStart {
                name, arguments, ..
            } => {
                self.progress("running", activity_of(&name, &arguments));
                self.steps().push(Step {
                    kind: "call".into(),
                    tool: name,
                    input: arguments.to_string(),
                    output: String::new(),
                    is_error: false,
                });
            }
            ChatEvent::ToolEnd {
                output, is_error, ..
            } => {
                if let Some(step) = self
                    .steps()
                    .iter_mut()
                    .rev()
                    .find(|s| s.kind == "call" && s.output.is_empty())
                {
                    step.output = short(&output, STEP_OUTPUT_CHARS);
                    step.is_error = is_error;
                }
            }
            _ => {}
        }
    }

    fn chat_state(&self) -> &crate::chat::ChatState {
        &self.state
    }

    fn registry(&self) -> &crate::tools::registry::ToolRegistry {
        self.parent.registry()
    }

    async fn connect_model(&self, reference: &str) -> Result<DynModel<Completion>, String> {
        self.parent.connect_model(reference).await
    }

    async fn approve(
        &self,
        tool: &ChatTool,
        input: &Value,
        reason: Option<&str>,
    ) -> crate::tools::registry::ApprovalDecision {
        self.parent.approve(tool, input, reason).await
    }

    fn notify(&self, title: &str, body: &str) {
        self.parent.notify(title, body);
    }

    fn app(&self) -> Option<&tauri::AppHandle> {
        self.parent.app()
    }
}

/// One worker: a turn of the app's own agent loop, in a temporary conversation of its own, with
/// the worker's brief as its whole system prompt and only the reading tools it may use. The same
/// hooks, vault, activity log, privacy filter and backup model apply as in a chat.
#[allow(clippy::too_many_arguments)]
async fn run_one(
    host: &Arc<dyn Host>,
    config_dir: &Path,
    reference: &str,
    tools: &[ChatTool],
    conversation_id: &str,
    call_id: &str,
    index: usize,
    task: Task,
    agent: Option<&crate::chat::agents::Agent>,
) -> Report {
    let worker = Arc::new(WorkerHost {
        parent: host.clone(),
        state: crate::chat::ChatState::default(),
        conversation_id: conversation_id.into(),
        call_id: call_id.into(),
        index,
        title: task.title.clone(),
        steps: Mutex::new(Vec::new()),
    });
    worker.progress("running", "Starting".into());
    let id = format!("{conversation_id}-worker-{}", uuid::Uuid::new_v4().simple());
    let finish = |report: Report| {
        let mut steps = worker.steps().clone();
        steps.push(Step {
            kind: "text".into(),
            tool: String::new(),
            input: String::new(),
            output: report.report.clone(),
            is_error: report.status != "done",
        });
        save_transcript(config_dir, call_id, index, &steps);
        // The worker's conversation and its saved long outputs are not kept.
        let _ = store::delete_conversation(config_dir, &id);
        let _ = std::fs::remove_dir_all(store::chat_dir(config_dir).join("tool-outputs").join(&id));
        worker.progress(
            report.status,
            if report.status == "done" {
                format!("Done after {} tool steps", report.steps)
            } else {
                short(&report.report, 120)
            },
        );
        report
    };
    let mut c = store::Conversation::new(id.clone(), None, Some(reference.to_string()));
    c.temporary = true;
    c.title = task.title.clone();
    c.agent_id = agent.map(|a| a.id.clone());
    if let Err(e) = store::save_conversation(config_dir, &c) {
        return finish(Report {
            title: task.title,
            status: "failed",
            report: e,
            steps: 0,
        });
    }
    let mut only: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
    only.push(crate::chat::engine::READ_TOOL_OUTPUT.into());
    let system = worker_preamble(&task, agent);
    let request = |text: String, only_tools: Vec<String>, max_steps: usize| TurnRequest {
        conversation_id: id.clone(),
        text,
        model: reference.to_string(),
        approval: ApprovalMode::Ask,
        temporary: true,
        max_steps: Some(max_steps),
        only_tools: Some(only_tools),
        system: Some(system.clone()),
        ..TurnRequest::default()
    };
    let dyn_worker: Arc<dyn Host> = worker.clone();
    let run = |request: TurnRequest| {
        let dyn_worker = dyn_worker.clone();
        let worker = worker.clone();
        let id = id.clone();
        async move {
            let cancel = worker.state.begin(&id).ok_or("the worker is busy")?;
            let outcome = crate::chat::engine::turn(&dyn_worker, &request, cancel).await;
            worker.state.finish(&id);
            outcome
        }
        .boxed()
    };
    let last_text = || {
        store::load_conversation(config_dir, &id)
            .ok()
            .and_then(|c| {
                c.messages
                    .last()
                    .filter(|m| matches!(m, Message::Assistant(_)))
                    .map(context::message_text)
            })
            .unwrap_or_default()
    };
    if let Err(e) = run(request(
        format!("Begin: {}", task.title),
        only,
        MAX_WORKER_STEPS + 1,
    ))
    .await
    {
        let steps = worker.steps().len();
        return finish(Report {
            title: task.title,
            status: "failed",
            report: e,
            steps,
        });
    }
    let mut text = last_text();
    if !is_report(&text) {
        // Out of tool calls, some models write one out as text instead of reporting. Ask once
        // more, plainly, for the report, with no tools to reach for.
        if run(request(
            "Stop researching. Write your report now from what you have already found, as plain text. Do not call or mention tools.".into(),
            Vec::new(),
            1,
        ))
        .await
        .is_ok()
        {
            text = last_text();
        }
    }
    let steps = worker.steps().len();
    if !is_report(&text) {
        return finish(Report {
            title: task.title,
            status: "failed",
            report: "The worker did not write a report.".into(),
            steps,
        });
    }
    finish(Report {
        title: task.title,
        status: "done",
        report: short(&text, MAX_REPORT_CHARS),
        steps,
    })
}

/// Runs every task at once; reports come back in task order. A task for a named agent runs
/// with that agent's instructions, its model, and only the reading tools it is allowed.
#[allow(clippy::too_many_arguments)]
pub async fn run_all(
    host: &Arc<dyn Host>,
    config_dir: &Path,
    model_reference: &str,
    offered: &[ChatTool],
    conversation_id: &str,
    call_id: &str,
    tasks: Vec<Task>,
    mode: Mode,
) -> Vec<Report> {
    let tools = worker_tools(offered);
    let team = crate::chat::agents::load(config_dir);
    let mut prepared = Vec::new();
    for mut task in tasks {
        let agent = task
            .agent
            .as_deref()
            .and_then(|n| crate::chat::agents::by_name(&team, n))
            .cloned();
        // The agent's own model when it can be reached, otherwise the chat's.
        let reference = match agent.as_ref().and_then(|a| a.model.clone()) {
            Some(own) if host.connect_model(&own).await.is_ok() => own,
            _ => model_reference.to_string(),
        };
        let allowed = task_tools(&tools, agent.as_ref(), &task.tools);
        if let Some(a) = &agent {
            task.title = format!("{}: {}", a.name, task.title);
        }
        prepared.push((task, agent, reference, allowed));
    }
    let run = |i: usize, task: Task, slot: usize| {
        let (_, agent, reference, allowed) = &prepared[slot];
        run_one(
            host,
            config_dir,
            reference,
            allowed,
            conversation_id,
            call_id,
            i,
            task,
            agent.as_ref(),
        )
    };
    match mode {
        Mode::Parallel => {
            join_all(
                prepared
                    .iter()
                    .enumerate()
                    .map(|(i, (task, ..))| run(i, task.clone(), i)),
            )
            .await
        }
        Mode::Sequential => {
            let mut reports: Vec<Report> = Vec::new();
            for (i, (task, ..)) in prepared.iter().enumerate() {
                let report = run(i, after(task, &reports), i).await;
                reports.push(report);
            }
            reports
        }
        Mode::Debate => {
            let n = prepared.len();
            let first = join_all(
                prepared
                    .iter()
                    .enumerate()
                    .map(|(i, (task, ..))| run(i, task.clone(), i)),
            )
            .await;
            if n < 2 {
                return first;
            }
            let second = join_all(
                prepared
                    .iter()
                    .enumerate()
                    .map(|(i, (task, ..))| run(n + i, rebuttal(task, i, &first), i)),
            )
            .await;
            first.into_iter().chain(second).collect()
        }
    }
}

/// One step of a worker, kept so the user can read what it did (Vibe's subagent history).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// `call` for a tool call and its result, `text` for what the worker wrote.
    pub kind: String,
    pub tool: String,
    pub input: String,
    pub output: String,
    pub is_error: bool,
}

/// Characters of each tool result kept in a transcript.
const STEP_OUTPUT_CHARS: usize = 2_000;

fn transcript_path(config_dir: &Path, call_id: &str, index: usize) -> Option<std::path::PathBuf> {
    let safe = !call_id.is_empty()
        && call_id.len() <= 200
        && call_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| {
        crate::chat::store::chat_dir(config_dir)
            .join("subagents")
            .join(format!("{call_id}-{index}.json"))
    })
}

fn save_transcript(config_dir: &Path, call_id: &str, index: usize, steps: &[Step]) {
    if let Some(path) = transcript_path(config_dir, call_id, index) {
        if let Err(e) = crate::chat::store::write_json(&path, &steps) {
            log::warn!("could not keep a worker's transcript: {e}");
        }
    }
}

/// What a worker did, step by step; empty when it was not kept.
pub fn transcript(config_dir: &Path, call_id: &str, index: usize) -> Vec<Step> {
    transcript_path(config_dir, call_id, index)
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// What the main conversation receives.
pub fn render(reports: &[Report]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({ "reports": reports })).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(title: &str, text: &str) -> Report {
        Report {
            title: title.into(),
            status: "done",
            report: text.into(),
            steps: 1,
        }
    }

    #[test]
    fn modes_pass_reports_along() {
        assert_eq!(Mode::parse(&json!({})), Mode::Parallel);
        assert_eq!(
            Mode::parse(&json!({"mode": "sequential"})),
            Mode::Sequential
        );
        assert_eq!(Mode::parse(&json!({"mode": "debate"})), Mode::Debate);
        let task = Task {
            title: "Pricing".into(),
            instructions: "Propose a price.".into(),
            agent: Some("Analyst".into()),
            tools: Vec::new(),
        };
        assert_eq!(after(&task, &[]).instructions, "Propose a price.");
        let next = after(
            &task,
            &[report("Researcher: Market", "Rivals charge 9 EUR.")],
        );
        assert!(
            next.instructions
                .contains("What the members before you reported")
                && next.instructions.contains("Rivals charge 9 EUR.")
        );
        let first = [
            report("A", "Charge 9."),
            report("B", "Charge 12."),
            report("C", "Charge 10."),
        ];
        let r = rebuttal(&task, 1, &first);
        assert_eq!(r.title, "Round 2 · Pricing");
        assert!(
            r.instructions.contains("Charge 9.")
                && r.instructions.contains("Charge 10.")
                && !r.instructions.contains("Charge 12."),
            "a member answers the others, not itself"
        );
        assert_eq!(r.agent.as_deref(), Some("Analyst"));
    }

    #[test]
    fn tasks_are_validated() {
        assert!(parse_tasks(&json!({})).is_err());
        assert!(parse_tasks(&json!({"tasks": []})).is_err());
        let six: Vec<_> = (0..6)
            .map(|i| json!({"title": i.to_string(), "instructions": "x"}))
            .collect();
        assert!(parse_tasks(&json!({ "tasks": six })).is_err());
        let ok = parse_tasks(
            &json!({"tasks": [{"title": "Prices", "instructions": "Find 2026 tariffs"}]}),
        )
        .unwrap();
        assert_eq!(ok[0].title, "Prices");
    }

    #[test]
    fn a_task_gets_only_the_tools_it_names() {
        let tool = |name: &str| ChatTool {
            name: name.into(),
            description: String::new(),
            parameters: json!({}),
            connector_id: None,
            tool_name: name.into(),
            read_only: true,
        };
        let workers = vec![
            tool("web_search"),
            tool("browser__fetch"),
            tool("use_skill"),
        ];
        let names = |tools: Vec<ChatTool>| tools.into_iter().map(|t| t.name).collect::<Vec<_>>();
        assert_eq!(names(task_tools(&workers, None, &[])).len(), 3);
        assert_eq!(
            names(task_tools(
                &workers,
                None,
                &["web_search".into(), "bash__run".into()]
            )),
            ["web_search"],
            "a task cannot ask for more than workers may use"
        );
        let tasks = parse_tasks(
            &json!({ "tasks": [{ "title": "t", "instructions": "i", "tools": ["web_search"] }] }),
        )
        .unwrap();
        assert_eq!(tasks[0].tools, vec!["web_search".to_string()]);
    }

    #[test]
    fn workers_only_get_reading_tools() {
        let tool = |name: &str| ChatTool {
            name: name.into(),
            description: String::new(),
            parameters: json!({}),
            connector_id: None,
            tool_name: name.into(),
            read_only: false,
        };
        let offered = vec![
            tool("ask_user"),
            tool("web_search"),
            tool("bash__run"),
            tool("browser__navigate"),
            tool("delegate"),
            tool("remember"),
        ];
        let names: Vec<String> = worker_tools(&offered).into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["web_search", "browser__navigate"]);
    }

    #[test]
    fn tool_calls_written_as_text_are_not_reports() {
        assert!(!is_report(""));
        assert!(!is_report(
            "[called tool browser__fetch with {\"url\":\"https://x\"}]"
        ));
        assert!(!is_report("Done."));
        assert!(is_report("The AI Act's obligations for high-risk systems apply from 2 August 2026, per Article 113."));
    }

    #[test]
    fn transcripts_are_kept_by_call() {
        let dir = tempfile::tempdir().unwrap();
        let steps = vec![Step {
            kind: "call".into(),
            tool: "web_search".into(),
            input: "{}".into(),
            output: "3 results".into(),
            is_error: false,
        }];
        save_transcript(dir.path(), "call_abc-1", 0, &steps);
        assert_eq!(transcript(dir.path(), "call_abc-1", 0), steps);
        assert!(transcript(dir.path(), "call_abc-1", 1).is_empty());
        assert!(transcript_path(dir.path(), "../x", 0).is_none());
    }

    #[test]
    fn reports_render_as_json_in_order() {
        let reports = vec![
            Report {
                title: "A".into(),
                status: "done",
                report: "found".into(),
                steps: 2,
            },
            Report {
                title: "B".into(),
                status: "failed",
                report: "no".into(),
                steps: 8,
            },
        ];
        let value: Value = serde_json::from_str(&render(&reports)).unwrap();
        assert_eq!(value["reports"][1]["title"], "B");
        assert_eq!(value["reports"][0]["status"], "done");
    }
}
