//! Sub-agents: independent workers the main assistant hands parts of a task to.
//!
//! The orchestrator-worker pattern, for context isolation and parallelism. Each worker starts
//! from a clean context holding only its task, uses read-only tools in a short loop of its own,
//! and returns a condensed report, so the main conversation receives a few hundred words per
//! task instead of every page a worker read. Workers run concurrently, can't ask the user
//! anything, can't change anything, and can't delegate further.

use std::path::Path;

use futures::future::join_all;
use rig_core::completion::message::{Message, ToolCall, ToolResultContent};
use rig_core::completion::CompletionRequest;
use rig_core::operation::Completion;
use rig_core::DynModel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::engine::{emit, ChatEvent};
use super::tools::{self, ApprovalMode, ChatTool, Outcome};
use super::{builtins, context};
use crate::tools::registry::ToolRegistry;

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
    "tool_results__read",
];

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub title: String,
    pub instructions: String,
    /// The agent to hand the task to, by name; a generic worker when absent.
    #[serde(default)]
    pub agent: Option<String>,
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
    offered.iter().filter(|t| WORKER_TOOLS.contains(&t.name.as_str())).cloned().collect()
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
    let notes: Vec<String> = earlier.iter().map(|r| format!("### {}\n{}", r.title, r.report.trim())).collect();
    Task {
        instructions: format!("{}\n\nWhat the members before you reported (build on it; do not repeat it):\n\n{}", task.instructions, notes.join("\n\n")),
        ..task.clone()
    }
}

/// A member's second-round task in a debate: the others' first answers, to answer.
pub fn rebuttal(task: &Task, own: usize, first: &[Report]) -> Task {
    let others: Vec<String> = first.iter().enumerate().filter(|(i, _)| *i != own).map(|(_, r)| format!("### {}\n{}", r.title, r.report.trim())).collect();
    Task {
        title: format!("Round 2 · {}", task.title),
        instructions: format!(
            "{}\n\nYou answered this already. Here are the other members' answers:\n\n{}\n\nReply with where you agree, where you disagree and why, and your revised answer. Keep what you still hold; change what they convinced you of.",
            task.instructions,
            others.join("\n\n")
        ),
        agent: task.agent.clone(),
    }
}

pub fn parse_tasks(args: &Value) -> Result<Vec<Task>, String> {
    let tasks: Vec<Task> = serde_json::from_value(args.get("tasks").cloned().unwrap_or(Value::Null))
        .map_err(|_| "`tasks` must be a list of {title, instructions}.".to_string())?;
    if tasks.is_empty() {
        return Err("Give at least one task.".into());
    }
    if tasks.len() > MAX_TASKS {
        return Err(format!("At most {MAX_TASKS} tasks at once."));
    }
    Ok(tasks)
}

fn worker_preamble(task: &Task, agent: Option<&super::agents::Agent>) -> String {
    let persona = agent.map_or(String::new(), |a| {
        format!("You are {}, an agent with this role: {}\n\n{}\n\n", a.name, a.description.trim(), a.instructions.trim())
    });
    persona + &format!(
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
    !text.is_empty() && !text.starts_with("[called tool") && !text.starts_with("<tool_call") && text.split_whitespace().count() >= 8
}

fn short(text: &str, max: usize) -> String {
    let text: String = text.chars().take(max).collect();
    text.trim().to_string()
}

/// One line saying what a worker is doing, for the progress row.
fn activity(call: &ToolCall) -> String {
    let args = Value::Object(call.function.arguments.clone());
    let detail = ["query", "url", "path"]
        .iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
        .unwrap_or_default();
    let what = match call.function.name.as_str() {
        builtins::WEB_SEARCH => "Searching",
        "browser__navigate" | "browser__fetch" => "Reading",
        "filesystem__fs_search" => "Searching files",
        "filesystem__fs_read" => "Reading file",
        builtins::USE_SKILL => "Loading skill",
        other => other,
    };
    format!("{what} {}", short(detail, 80)).trim().to_string()
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    app: &AppHandle,
    config_dir: &Path,
    model: &DynModel<Completion>,
    tools: &[ChatTool],
    conversation_id: &str,
    call_id: &str,
    index: usize,
    task: Task,
    agent: Option<&super::agents::Agent>,
) -> Report {
    let progress = |status: &str, activity: String| {
        emit(
            app,
            ChatEvent::Subagent {
                conversation_id: conversation_id.into(),
                call_id: call_id.into(),
                index,
                title: task.title.clone(),
                status: status.into(),
                activity,
            },
        );
    };
    progress("running", "Starting".into());
    let registry = app.state::<ToolRegistry>();
    let definitions: Vec<_> = tools.iter().filter_map(ChatTool::definition).collect();
    let system = worker_preamble(&task, agent);
    let mut messages = vec![Message::user(format!("Begin: {}", task.title))];
    let mut steps: Vec<Step> = Vec::new();
    let finish = |steps: &mut Vec<Step>, report: Report| {
        steps.push(Step { kind: "text".into(), tool: String::new(), input: String::new(), output: report.report.clone(), is_error: report.status != "done" });
        save_transcript(config_dir, call_id, index, steps);
        report
    };

    for step in 0..=MAX_WORKER_STEPS {
        let last = step == MAX_WORKER_STEPS;
        let request_messages = context::request_messages(&messages, None);
        let Some((prompt, history)) = request_messages.split_last() else { break };
        let mut request = CompletionRequest::new(prompt.clone())
            .messages(history.iter().cloned())
            .preamble(if last { format!("{system}\n\nNo more tool calls: write the report now.") } else { system.clone() });
        if !last {
            request = request.tools(definitions.iter().cloned());
        }
        let response = match model.call(request).await {
            Ok(r) => r,
            Err(e) => {
                progress("failed", short(&e.to_string(), 120));
                return finish(&mut steps, Report { title: task.title.clone(), status: "failed", report: e.to_string(), steps: step });
            }
        };
        let calls: Vec<ToolCall> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            messages.push(message);
        }
        if calls.is_empty() {
            let mut text = response.text();
            if !is_report(&text) {
                // Out of tool calls, some models write one out as text instead of reporting.
                // Ask once more, plainly, for the report.
                messages.push(Message::user("Stop researching. Write your report now from what you have already found, as plain text. Do not call or mention tools."));
                let request_messages = context::request_messages(&messages, None);
                if let Some((prompt, history)) = request_messages.split_last() {
                    let request = CompletionRequest::new(prompt.clone()).messages(history.iter().cloned()).preamble(system.clone());
                    if let Ok(retry) = model.call(request).await {
                        text = retry.text();
                    }
                }
            }
            if !is_report(&text) {
                progress("failed", "No report".into());
                return finish(&mut steps, Report { title: task.title.clone(), status: "failed", report: "The worker did not write a report.".into(), steps: step });
            }
            let report = short(&text, MAX_REPORT_CHARS);
            progress("done", format!("Done after {step} tool steps"));
            return finish(&mut steps, Report { title: task.title.clone(), status: "done", report, steps: step });
        }
        progress("running", activity(&calls[0]));
        let outcomes = join_all(calls.iter().map(|call| {
            let name = call.function.name.to_string();
            let args = Value::Object(call.function.arguments.clone());
            let registry = &registry;
            async move {
                if builtins::is_builtin(&name) {
                    builtins::run(config_dir, &name, &args).await
                } else if let Some(tool) = tools.iter().find(|t| t.name == name) {
                    tools::execute(app, registry, tool, args, ApprovalMode::Ask, conversation_id).await
                } else {
                    Outcome { text: format!("{name} is not available to workers."), is_error: true }
                }
            }
        }))
        .await;
        for (call, outcome) in calls.iter().zip(&outcomes) {
            steps.push(Step {
                kind: "call".into(),
                tool: call.function.name.to_string(),
                input: serde_json::to_string(&call.function.arguments).unwrap_or_default(),
                output: short(&outcome.text, STEP_OUTPUT_CHARS),
                is_error: outcome.is_error,
            });
        }
        let results = calls
            .iter()
            .zip(outcomes)
            .map(|(call, outcome)| {
                let mut result = call.result(vec![ToolResultContent::text(outcome.text)]);
                result.is_error = outcome.is_error;
                result
            })
            .collect();
        messages.push(Message::tool_results(results));
    }
    progress("failed", "Ran out of steps".into());
    finish(&mut steps, Report { title: task.title, status: "failed", report: "The worker ran out of steps without a report.".into(), steps: MAX_WORKER_STEPS })
}

/// Runs every task at once; reports come back in task order. A task for a named agent runs
/// with that agent's instructions, its model, and only the reading tools it is allowed.
#[allow(clippy::too_many_arguments)]
pub async fn run_all(
    app: &AppHandle,
    config_dir: &Path,
    model: &DynModel<Completion>,
    offered: &[ChatTool],
    conversation_id: &str,
    call_id: &str,
    tasks: Vec<Task>,
    mode: Mode,
) -> Vec<Report> {
    let tools = worker_tools(offered);
    let team = super::agents::load(config_dir);
    let mut prepared = Vec::new();
    for mut task in tasks {
        let agent = task.agent.as_deref().and_then(|n| super::agents::by_name(&team, n)).cloned();
        let own = match agent.as_ref().and_then(|a| a.model.clone()) {
            Some(reference) => super::engine::connect_model(config_dir, &reference).await.ok(),
            None => None,
        };
        let allowed: Vec<ChatTool> = tools.iter().filter(|t| agent.as_ref().is_none_or(|a| super::agents::allows(a, &t.name))).cloned().collect();
        if let Some(a) = &agent {
            task.title = format!("{}: {}", a.name, task.title);
        }
        prepared.push((task, agent, own, allowed));
    }
    let run = |i: usize, task: Task, slot: usize| {
        let (_, agent, own, allowed) = &prepared[slot];
        run_one(app, config_dir, own.as_ref().unwrap_or(model), allowed, conversation_id, call_id, i, task, agent.as_ref())
    };
    match mode {
        Mode::Parallel => join_all(prepared.iter().enumerate().map(|(i, (task, ..))| run(i, task.clone(), i))).await,
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
            let first = join_all(prepared.iter().enumerate().map(|(i, (task, ..))| run(i, task.clone(), i))).await;
            if n < 2 {
                return first;
            }
            let second = join_all(prepared.iter().enumerate().map(|(i, (task, ..))| run(n + i, rebuttal(task, i, &first), i))).await;
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
    let safe = !call_id.is_empty() && call_id.len() <= 200 && call_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| super::store::chat_dir(config_dir).join("subagents").join(format!("{call_id}-{index}.json")))
}

fn save_transcript(config_dir: &Path, call_id: &str, index: usize, steps: &[Step]) {
    if let Some(path) = transcript_path(config_dir, call_id, index) {
        if let Err(e) = super::store::write_json(&path, &steps) {
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
        Report { title: title.into(), status: "done", report: text.into(), steps: 1 }
    }

    #[test]
    fn modes_pass_reports_along() {
        assert_eq!(Mode::parse(&json!({})), Mode::Parallel);
        assert_eq!(Mode::parse(&json!({"mode": "sequential"})), Mode::Sequential);
        assert_eq!(Mode::parse(&json!({"mode": "debate"})), Mode::Debate);
        let task = Task { title: "Pricing".into(), instructions: "Propose a price.".into(), agent: Some("Analyst".into()) };
        assert_eq!(after(&task, &[]).instructions, "Propose a price.");
        let next = after(&task, &[report("Researcher: Market", "Rivals charge 9 EUR.")]);
        assert!(next.instructions.contains("What the members before you reported") && next.instructions.contains("Rivals charge 9 EUR."));
        let first = [report("A", "Charge 9."), report("B", "Charge 12."), report("C", "Charge 10.")];
        let r = rebuttal(&task, 1, &first);
        assert_eq!(r.title, "Round 2 · Pricing");
        assert!(r.instructions.contains("Charge 9.") && r.instructions.contains("Charge 10.") && !r.instructions.contains("Charge 12."), "a member answers the others, not itself");
        assert_eq!(r.agent.as_deref(), Some("Analyst"));
    }

    #[test]
    fn tasks_are_validated() {
        assert!(parse_tasks(&json!({})).is_err());
        assert!(parse_tasks(&json!({"tasks": []})).is_err());
        let six: Vec<_> = (0..6).map(|i| json!({"title": i.to_string(), "instructions": "x"})).collect();
        assert!(parse_tasks(&json!({ "tasks": six })).is_err());
        let ok = parse_tasks(&json!({"tasks": [{"title": "Prices", "instructions": "Find 2026 tariffs"}]})).unwrap();
        assert_eq!(ok[0].title, "Prices");
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
        let offered = vec![tool("ask_user"), tool("web_search"), tool("bash__run"), tool("browser__navigate"), tool("delegate"), tool("remember")];
        let names: Vec<String> = worker_tools(&offered).into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["web_search", "browser__navigate"]);
    }

    #[test]
    fn tool_calls_written_as_text_are_not_reports() {
        assert!(!is_report(""));
        assert!(!is_report("[called tool browser__fetch with {\"url\":\"https://x\"}]"));
        assert!(!is_report("Done."));
        assert!(is_report("The AI Act's obligations for high-risk systems apply from 2 August 2026, per Article 113."));
    }

    #[test]
    fn transcripts_are_kept_by_call() {
        let dir = tempfile::tempdir().unwrap();
        let steps = vec![Step { kind: "call".into(), tool: "web_search".into(), input: "{}".into(), output: "3 results".into(), is_error: false }];
        save_transcript(dir.path(), "call_abc-1", 0, &steps);
        assert_eq!(transcript(dir.path(), "call_abc-1", 0), steps);
        assert!(transcript(dir.path(), "call_abc-1", 1).is_empty());
        assert!(transcript_path(dir.path(), "../x", 0).is_none());
    }

    #[test]
    fn reports_render_as_json_in_order() {
        let reports = vec![
            Report { title: "A".into(), status: "done", report: "found".into(), steps: 2 },
            Report { title: "B".into(), status: "failed", report: "no".into(), steps: 8 },
        ];
        let value: Value = serde_json::from_str(&render(&reports)).unwrap();
        assert_eq!(value["reports"][1]["title"], "B");
        assert_eq!(value["reports"][0]["status"], "done");
    }
}
