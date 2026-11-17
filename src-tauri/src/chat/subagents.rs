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

fn worker_preamble(task: &Task) -> String {
    format!(
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
    let system = worker_preamble(&task);
    let mut messages = vec![Message::user(format!("Begin: {}", task.title))];

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
                return Report { title: task.title.clone(), status: "failed", report: e.to_string(), steps: step };
            }
        };
        let calls: Vec<ToolCall> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            messages.push(message);
        }
        if calls.is_empty() {
            let report = short(&response.text(), MAX_REPORT_CHARS);
            progress("done", format!("Done after {step} tool steps"));
            return Report { title: task.title.clone(), status: "done", report, steps: step };
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
    Report { title: task.title, status: "failed", report: "The worker ran out of steps without a report.".into(), steps: MAX_WORKER_STEPS }
}

/// Runs every task at once; reports come back in task order.
pub async fn run_all(
    app: &AppHandle,
    config_dir: &Path,
    model: &DynModel<Completion>,
    offered: &[ChatTool],
    conversation_id: &str,
    call_id: &str,
    tasks: Vec<Task>,
) -> Vec<Report> {
    let tools = worker_tools(offered);
    join_all(
        tasks
            .into_iter()
            .enumerate()
            .map(|(i, task)| run_one(app, config_dir, model, &tools, conversation_id, call_id, i, task)),
    )
    .await
}

/// What the main conversation receives.
pub fn render(reports: &[Report]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({ "reports": reports })).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
