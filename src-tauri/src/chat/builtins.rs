//! Tools the app itself provides, beside the connectors: web search, memory and skills.

use std::path::Path;

use serde_json::{json, Value};

use super::tools::{ChatTool, Outcome};
use super::{memory, search, skills};

pub const WEB_SEARCH: &str = "web_search";
pub const REMEMBER: &str = "remember";
pub const FORGET: &str = "forget";
pub const USE_SKILL: &str = "use_skill";
pub const UPDATE_PLAN: &str = "update_plan";
/// Run by the engine, which holds the model the workers need.
pub const DELEGATE: &str = "delegate";
/// Run by the engine, which knows the conversation the canvas belongs to.
pub const CANVAS: &str = "canvas";
pub const SCHEDULE_TASK: &str = "schedule_task";

fn tool(name: &str, description: &str, parameters: Value) -> ChatTool {
    ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only: true,
    }
}

/// The built-in tools on offer: memory only while it is on, `use_skill` only with skills enabled.
pub fn definitions(config_dir: &Path) -> Vec<ChatTool> {
    let mut out = vec![tool(
        WEB_SEARCH,
        "Search the web. Use it for current events, prices, versions, people, places and any fact \
         you are not sure of. Returns titles, URLs and snippets; open a result with the browser \
         tools when the snippet is not enough.",
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search terms, as you would type them." },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 10, "default": 6 }
            },
            "required": ["query"]
        }),
    )];
    out.push(tool(
        UPDATE_PLAN,
        "Show the user your plan for a multi-step task as a checklist, and update it as steps \
         finish. Send the whole list each time.",
        json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done"] }
                        },
                        "required": ["text", "status"]
                    }
                }
            },
            "required": ["steps"]
        }),
    ));
    out.push(tool(
        DELEGATE,
        "Hand independent parts of a broad task to sub-agents that work in parallel, each with a \
         clean context and read-only tools (web search, browsing, reading files). Each returns a \
         short report with sources. Use it for research spanning several topics, sources or \
         files; do small lookups yourself. Write each task so it stands alone: the worker sees \
         nothing of this conversation.",
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": super::subagents::MAX_TASKS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "A few words." },
                            "instructions": { "type": "string", "description": "What to find or check, and what to report." }
                        },
                        "required": ["title", "instructions"]
                    }
                }
            },
            "required": ["tasks"]
        }),
    ));
    out.push(tool(
        CANVAS,
        "Write a document the user will keep, read at length or edit: a report, letter, plan, \
         article, README or code file. Creates a canvas, or with `id` replaces an existing one's \
         text with a new version. Send the complete text each time, in Markdown (code in fenced \
         blocks). Keep your chat reply to a sentence or two about what you wrote.",
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The canvas to update; omit to create one." },
                "title": { "type": "string" },
                "content": { "type": "string", "description": "The full document, in Markdown." }
            },
            "required": ["content"]
        }),
    ));
    out.push(tool(
        SCHEDULE_TASK,
        "Schedule a prompt to run automatically, when the user asks for something recurring or \
         later (\"every Monday at 9, send me…\", \"tomorrow at 8 remind me…\"). Each run \
         starts a new conversation with the prompt. Write the prompt so it stands alone. Times \
         are the user's local time.",
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A few words for the task list." },
                "prompt": { "type": "string", "description": "What to do on each run, written to stand alone." },
                "frequency": { "type": "string", "enum": ["once", "daily", "weekly", "monthly"] },
                "time": { "type": "string", "description": "24-hour local time, HH:MM." },
                "days": { "type": "array", "items": { "type": "string" }, "description": "Weekly: day names, e.g. [\"monday\"]." },
                "day_of_month": { "type": "integer", "minimum": 1, "maximum": 31 },
                "date": { "type": "string", "description": "Once: YYYY-MM-DD." },
                "agent": { "type": "string", "description": "Name of an agent to run it as, if the user named one." }
            },
            "required": ["title", "prompt", "frequency", "time"]
        }),
    ));
    if memory::load(config_dir).enabled {
        out.push(tool(
            REMEMBER,
            "Save one lasting fact about the user for future conversations.",
            json!({
                "type": "object",
                "properties": { "fact": { "type": "string", "description": "A short, self-contained fact." } },
                "required": ["fact"]
            }),
        ));
        out.push(tool(
            FORGET,
            "Delete a saved fact that is wrong or that the user asks you to forget.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The id shown next to the fact." } },
                "required": ["id"]
            }),
        ));
    }
    if skills::list(config_dir).iter().any(|s| s.enabled) {
        out.push(tool(
            USE_SKILL,
            "Load a skill's instructions by name, then follow them.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
        ));
    }
    out
}

pub fn is_builtin(name: &str) -> bool {
    matches!(name, WEB_SEARCH | REMEMBER | FORGET | USE_SKILL | UPDATE_PLAN | SCHEDULE_TASK)
}

fn ok(text: impl Into<String>) -> Outcome {
    Outcome { text: text.into(), is_error: false }
}

fn err(text: impl Into<String>) -> Outcome {
    Outcome { text: text.into(), is_error: true }
}

pub async fn run(config_dir: &Path, name: &str, args: &Value) -> Outcome {
    let str_arg = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    match name {
        WEB_SEARCH => {
            let query = str_arg("query");
            if query.is_empty() {
                return err("Give a search query.");
            }
            let max = args.get("max_results").and_then(Value::as_u64).unwrap_or(6).clamp(1, 10) as usize;
            match search::search(&query, max).await {
                Ok(hits) => ok(serde_json::to_string_pretty(&json!({ "query": query, "results": hits })).unwrap_or_default()),
                Err(e) => err(e),
            }
        }
        REMEMBER => {
            let mut memories = memory::load(config_dir);
            match memory::add(&mut memories, &str_arg("fact"), "assistant") {
                Ok(m) => match memory::save(config_dir, &memories) {
                    Ok(()) => ok(format!("Saved as {}.", &m.id[..8])),
                    Err(e) => err(e),
                },
                Err(e) => err(e),
            }
        }
        FORGET => {
            let mut memories = memory::load(config_dir);
            let Some(id) = memory::resolve(&memories, &str_arg("id")).map(|m| m.id.clone()) else {
                return err("No saved fact has that id.");
            };
            memory::remove(&mut memories, &id);
            match memory::save(config_dir, &memories) {
                Ok(()) => ok("Forgotten."),
                Err(e) => err(e),
            }
        }
        SCHEDULE_TASK => schedule(config_dir, args),
        UPDATE_PLAN => {
            let steps = args.get("steps").and_then(Value::as_array).map_or(0, Vec::len);
            if steps == 0 {
                err("Give the plan's steps.")
            } else {
                ok(format!("Plan shown to the user ({steps} steps)."))
            }
        }
        USE_SKILL => match skills::find(config_dir, &str_arg("name")) {
            Some(skill) if skill.enabled => ok(format!("# Skill: {}\n\n{}", skill.name, skill.body)),
            Some(_) => err("That skill is turned off."),
            None => err("There is no skill with that name."),
        },
        _ => err(format!("Unknown built-in tool {name}.")),
    }
}

fn schedule(config_dir: &Path, args: &Value) -> Outcome {
    use super::tasks::{self, Frequency};
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let time = s("time");
    let frequency = match s("frequency").as_str() {
        "once" => Frequency::Once { date: s("date"), time },
        "daily" => Frequency::Daily { time },
        "weekly" => {
            let days: Vec<u8> = args
                .get("days")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).filter_map(tasks::weekday_index).collect())
                .unwrap_or_default();
            Frequency::Weekly { days, time }
        }
        "monthly" => Frequency::Monthly { day: args.get("day_of_month").and_then(Value::as_u64).unwrap_or(1).min(31) as u8, time },
        other => return err(format!("Unknown frequency `{other}`.")),
    };
    let agent_id = match s("agent") {
        name if name.is_empty() => None,
        name => match super::agents::load(config_dir).into_iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
            Some(agent) => Some(agent.id),
            None => return err(format!("There is no agent named {name}.")),
        },
    };
    let task = tasks::Task {
        id: uuid::Uuid::new_v4().to_string(),
        title: s("title"),
        prompt: s("prompt"),
        frequency: frequency.clone(),
        agent_id,
        model: None,
        enabled: true,
        allow_changes: false,
        created_at: chrono::Utc::now(),
        last_run: None,
        last_conversation: None,
        last_status: None,
        next_run: None,
    };
    let id = task.id.clone();
    match tasks::save(config_dir, task) {
        Ok(saved) => {
            let next = saved.iter().find(|t| t.id == id).and_then(|t| t.next_run);
            ok(json!({
                "scheduled": tasks::describe(&frequency),
                "next_run": next.map(|n| n.with_timezone(&chrono::Local).format("%A %-d %B %Y, %H:%M").to_string()),
                "note": "Tell the user it is scheduled; they can pause or edit it on the Scheduled page."
            })
            .to_string())
        }
        Err(e) => err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scheduling_from_chat_creates_a_task() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(dir.path(), SCHEDULE_TASK, &json!({"title": "AI news", "prompt": "Brief me", "frequency": "weekly", "time": "09:00", "days": ["monday"]})).await;
        assert!(!out.is_error, "{}", out.text);
        assert!(out.text.contains("Every Monday at 09:00"));
        assert_eq!(super::super::tasks::load(dir.path()).len(), 1);
        let bad = run(dir.path(), SCHEDULE_TASK, &json!({"title": "x", "prompt": "y", "frequency": "hourly", "time": "09:00"})).await;
        assert!(bad.is_error);
    }

    #[tokio::test]
    async fn memory_tools_save_and_forget() {
        let dir = tempfile::tempdir().unwrap();
        let saved = run(dir.path(), REMEMBER, &json!({"fact": "Lives in Porto"})).await;
        assert!(!saved.is_error, "{}", saved.text);
        let memories = memory::load(dir.path());
        assert_eq!(memories.items.len(), 1);
        let short = &memories.items[0].id[..8];
        let forgotten = run(dir.path(), FORGET, &json!({"id": short})).await;
        assert!(!forgotten.is_error, "{}", forgotten.text);
        assert!(memory::load(dir.path()).items.is_empty());
        assert!(run(dir.path(), FORGET, &json!({"id": "nope"})).await.is_error);
    }

    #[tokio::test]
    async fn skills_load_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = run(dir.path(), USE_SKILL, &json!({"name": "write-email"})).await;
        assert!(!loaded.is_error);
        assert!(loaded.text.starts_with("# Skill: write-email"));
        assert!(run(dir.path(), USE_SKILL, &json!({"name": "missing"})).await.is_error);
    }

    #[test]
    fn definitions_follow_settings() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).collect();
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, SCHEDULE_TASK, REMEMBER, FORGET, USE_SKILL]);
        memory::save(dir.path(), &memory::Memories { enabled: false, items: vec![] }).unwrap();
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).collect();
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, SCHEDULE_TASK, USE_SKILL]);
    }
}
