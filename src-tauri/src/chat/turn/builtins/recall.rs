//! The built-ins that save and find what the user told the assistant: memories, knowledge pages, documents and earlier chats.

use super::*;

/// `REMEMBER`.
pub(super) async fn remember(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let mut memories = memory::load(config_dir);
    let filing = memory::Filing {
        category: str_arg("category"),
        importance: match str_arg("importance").as_str() {
            "low" => Some(1),
            "high" => Some(3),
            _ => Some(2),
        },
        inferred: args
            .get("inferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        conversation: Some(str_arg("conversation")).filter(|c| !c.is_empty()),
        scope: None,
    };
    let scope = match str_arg("scope").as_str() {
        "project" => Some(str_arg("_project"))
            .filter(|p| !p.is_empty())
            .map(|p| format!("project:{p}")),
        "agent" => Some(str_arg("_agent"))
            .filter(|a| !a.is_empty())
            .map(|a| format!("agent:{a}")),
        _ => None,
    };
    let filing = memory::Filing { scope, ..filing };
    if let Some(reason) = memory::refuse_sensitive(&memories, &str_arg("fact")) {
        return err(reason);
    }
    match memory::add_filed(&mut memories, &str_arg("fact"), "assistant", filing) {
        Ok(m) => match memory::save(config_dir, &memories) {
            Ok(()) => ok(format!("Saved as {}.", &m.id[..8])),
            Err(e) => err(e),
        },
        Err(e) => err(e),
    }
}

/// `FORGET`.
pub(super) async fn forget(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
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

/// `KNOWLEDGE_WRITE`.
pub(super) async fn knowledge_write(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let sources: Vec<String> = args
        .get("sources")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let id = str_arg("id");
    if let Some(reason) = memory::refuse_sensitive(
        &memory::load(config_dir),
        &format!("{}\n{}", str_arg("title"), str_arg("content")),
    ) {
        return err(reason);
    }
    match crate::chat::knowledge::write(
        config_dir,
        (!id.is_empty()).then_some(id.as_str()),
        &str_arg("title"),
        &str_arg("content"),
        "assistant",
        &sources,
    ) {
        Ok(page) => ok(json!({ "id": page.id, "title": page.title, "saved": true }).to_string()),
        Err(e) => err(e),
    }
}

/// `KNOWLEDGE_READ`.
pub(super) async fn knowledge_read(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let pages = crate::chat::knowledge::load(config_dir);
    match crate::chat::knowledge::find(&pages, &str_arg("page")) {
        Some(page) => ok(format!(
            "# {}\n\n{}\n\nSources: {}",
            page.title,
            page.body,
            if page.sources.is_empty() {
                "none".into()
            } else {
                page.sources.join("; ")
            }
        )),
        None => err("There is no knowledge page with that id or title."),
    }
}

/// `KNOWLEDGE_SEARCH`.
pub(super) async fn knowledge_search(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let pages = crate::chat::knowledge::load(config_dir);
    let hits: Vec<Value> = crate::chat::knowledge::rank(&pages, &str_arg("query"))
            .into_iter()
            .take(5)
            .map(|(p, _)| json!({ "id": &p.id[..8], "title": p.title, "excerpt": crate::support::text::clip(&p.body, 400) }))
            .collect();
    if hits.is_empty() {
        ok("No knowledge page matches.")
    } else {
        ok(serde_json::to_string_pretty(&hits).unwrap_or_default())
    }
}

/// `READ_DOCUMENT`.
pub(super) async fn read_document(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let wanted = str_arg("document");
    let in_library = str_arg("library");
    let part = args.get("part").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
    match crate::chat::libraries::read_document(config_dir, &in_library, &wanted, part) {
        Ok(text) => ok(text),
        Err(e) => err(e),
    }
}

/// `SEARCH_CHATS`.
pub(super) async fn search_chats(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a search query.");
    }
    let hits = crate::chat::index::search_chats(config_dir, &query);
    if hits.is_empty() {
        return ok(json!({ "query": query, "conversations": [], "note": "No earlier conversation matched; try other words." }).to_string());
    }
    let rows: Vec<Value> = hits
            .iter()
            .map(|h| json!({ "id": h.id, "title": h.title, "date": h.updated_at.format("%Y-%m-%d").to_string(), "snippet": h.snippet }))
            .collect();
    ok(
        serde_json::to_string_pretty(&json!({ "query": query, "conversations": rows }))
            .unwrap_or_default(),
    )
}

/// `MEMORY_SEARCH`.
pub(super) async fn memory_search(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let memories = memory::load(config_dir);
    let project = Some(str_arg("_project")).filter(|p| !p.is_empty());
    let agent = Some(str_arg("_agent")).filter(|a| !a.is_empty());
    let found: Vec<String> = memory::search(
        &memories,
        &str_arg("query"),
        project.as_deref(),
        agent.as_deref(),
    )
    .into_iter()
    .take(15)
    .map(|m| format!("- {}: {}", &m.id[..8.min(m.id.len())], m.text))
    .collect();
    if found.is_empty() {
        ok("No saved fact matches.")
    } else {
        ok(found.join("\n"))
    }
}
