//! Tools the app itself provides, beside the connectors: web search, memory and skills.

use std::path::Path;

use serde_json::{json, Value};

use super::tools::{ChatTool, Outcome};
use super::{memory, search, skills};

pub const WEB_SEARCH: &str = "web_search";
pub const REMEMBER: &str = "remember";
pub const FORGET: &str = "forget";
pub const USE_SKILL: &str = "use_skill";

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
    matches!(name, WEB_SEARCH | REMEMBER | FORGET | USE_SKILL)
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
        assert_eq!(names, [WEB_SEARCH, REMEMBER, FORGET, USE_SKILL]);
        memory::save(dir.path(), &memory::Memories { enabled: false, items: vec![] }).unwrap();
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).collect();
        assert_eq!(names, [WEB_SEARCH, USE_SKILL]);
    }
}
