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
    matches!(name, WEB_SEARCH | REMEMBER | FORGET | USE_SKILL | UPDATE_PLAN)
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, REMEMBER, FORGET, USE_SKILL]);
        memory::save(dir.path(), &memory::Memories { enabled: false, items: vec![] }).unwrap();
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).collect();
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, USE_SKILL]);
    }
}
