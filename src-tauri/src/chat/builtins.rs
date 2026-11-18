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
pub const SEARCH_LIBRARY: &str = "search_library";
pub const READ_DOCUMENT: &str = "read_document";
pub const SAVE_SKILL: &str = "save_skill";
pub const KNOWLEDGE_WRITE: &str = "knowledge_write";
pub const KNOWLEDGE_READ: &str = "knowledge_read";
pub const KNOWLEDGE_SEARCH: &str = "knowledge_search";
pub const SEARCH_CHATS: &str = "search_chats";
pub const READ_CHAT: &str = "read_chat";
pub const NOTIFY: &str = "notify";

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
                "max_results": { "type": "integer", "minimum": 1, "maximum": 10, "default": 6 },
                "recency": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Only results from the past day, week, month or year: for news and anything that changes." },
                "sites": { "type": "array", "items": { "type": "string" }, "maxItems": 5, "description": "Only results from these sites, e.g. [\"docs.rs\", \"github.com\"]: for official docs or a source the user named." }
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
                            "instructions": { "type": "string", "description": "What to find or check, and what to report." },
                            "agent": { "type": "string", "description": "Optional: the name of an agent from your team to do this task." }
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
         article, README or code file, or an interactive mini app (calculator, game, chart, \
         form, dashboard). Creates a canvas, or with `id` replaces an existing one's \
         text with a new version. Send the complete text each time, in Markdown (code in fenced \
         blocks). Keep your chat reply to a sentence or two about what you wrote.",
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The canvas to update; omit to create one." },
                "kind": {
                    "type": "string",
                    "enum": ["document", "app", "sheet"],
                    "description": "document: Markdown. sheet: a spreadsheet table as CSV (first row is the header), which the user edits as a grid and downloads as Excel; for several sheets, start each with a line `## Sheet: Name` followed by its CSV. app: an interactive mini app, as one complete self-contained HTML document with inline CSS and JS (libraries only from cdn.jsdelivr.net, unpkg.com or cdnjs.cloudflare.com); it runs sandboxed with a live preview."
                },
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
         starts a new conversation with the prompt (or, with `in_this_chat`, continues this \
         one). Write the prompt so it stands alone. Times are the user's local time.",
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A few words for the task list." },
                "prompt": { "type": "string", "description": "What to do on each run, written to stand alone." },
                "frequency": { "type": "string", "enum": ["once", "daily", "weekly", "monthly", "yearly"] },
                "time": { "type": "string", "description": "24-hour local time, HH:MM." },
                "days": { "type": "array", "items": { "type": "string" }, "description": "Weekly: day names, e.g. [\"monday\"]." },
                "month": { "type": "integer", "minimum": 1, "maximum": 12, "description": "For yearly: the month, 1–12." },
                "day_of_month": { "type": "integer", "minimum": 1, "maximum": 31 },
                "date": { "type": "string", "description": "Once: YYYY-MM-DD." },
                "agent": { "type": "string", "description": "Name of an agent to run it as, if the user named one." },
                "in_this_chat": { "type": "boolean", "description": "Run in this conversation, with its context, instead of a new one: for follow-ups on what is being discussed here (\"check this again tomorrow\")." }
            },
            "required": ["title", "prompt", "frequency", "time"]
        }),
    ));
    let libraries = super::libraries::list(config_dir);
    if libraries.iter().any(|l| !l.documents.is_empty()) {
        out.push(tool(
            SEARCH_LIBRARY,
            "Search the user's document libraries and get the most relevant passages with their \
             document names. Use specific keywords; search again with other words if the first \
             results miss.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "library": { "type": "string", "description": "A library name, to search only that one." },
                    "since": { "type": "string", "description": "Only documents changed on or after this: a date (2026-03-01) or a span back from today (30d, 6m, 1y). For questions about recent or current versions." }
                },
                "required": ["query"]
            }),
        ));
    }
    if libraries.iter().any(|l| !l.documents.is_empty()) {
        out.push(tool(
            READ_DOCUMENT,
            "Read a whole library document, when passages from search_library are not enough \
             (a contract to check end to end, a report to summarize). Long documents come in parts: \
             ask for the next part with `part`.",
            json!({
                "type": "object",
                "properties": {
                    "document": { "type": "string", "description": "The document's name as search_library shows it." },
                    "library": { "type": "string", "description": "The library, when names repeat." },
                    "part": { "type": "integer", "minimum": 1, "default": 1 }
                },
                "required": ["document"]
            }),
        ));
    }
    if memory::load(config_dir).enabled {
        out.push(tool(
            KNOWLEDGE_WRITE,
            "Create or update a knowledge page: lasting, detailed context worth keeping across \
             conversations (a project, a client, a process, how the user wants something done). \
             Updating replaces the body, so include what should stay. Name your sources.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "content": { "type": "string", "description": "The whole page, in Markdown." },
                    "id": { "type": "string", "description": "The page to update; omit to create, or to update by title." },
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "Where this came from: a conversation, document or URL." }
                },
                "required": ["title", "content"]
            }),
        ));
        out.push(tool(
            KNOWLEDGE_READ,
            "Read a knowledge page in full, by id or title.",
            json!({ "type": "object", "properties": { "page": { "type": "string" } }, "required": ["page"] }),
        ));
        out.push(tool(
            KNOWLEDGE_SEARCH,
            "Search the user's knowledge pages; returns the best matching pages with an excerpt.",
            json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }),
        ));
        // Earlier conversations, as Open WebUI's history search: years of chats stay usable.
        out.push(tool(
            SEARCH_CHATS,
            "Search the user's earlier conversations by words in their titles and messages. Use it \
             when the user refers to something discussed before (\"what did we decide about…\", \
             \"like last time\"). Returns conversation ids, titles, dates and a snippet; read one \
             in full with read_chat.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "A few distinctive words." } },
                "required": ["query"]
            }),
        ));
        out.push(tool(
            READ_CHAT,
            "Read an earlier conversation's messages, by the id search_chats returned.",
            json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
        ));
        out.push(tool(
            REMEMBER,
            "Save one lasting fact about the user for future conversations.",
            json!({
                "type": "object",
                "properties": {
                    "fact": { "type": "string", "description": "A short, self-contained fact." },
                    "category": { "type": "string", "enum": ["preference", "work", "person", "project", "habit", "fact"], "description": "What it is about." },
                    "importance": { "type": "string", "enum": ["low", "normal", "high"], "description": "high: keep it in mind always (a strong preference, a constraint); low: a passing detail." },
                    "inferred": { "type": "boolean", "description": "true when you concluded it rather than the user saying it." }
                },
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
    out.push(tool(
        SAVE_SKILL,
        "Save a reusable skill the user asked for and approved: instructions you will follow \
         whenever a request matches its description. Overwrites a skill of your own with the \
         same name.",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Lowercase letters, digits and hyphens, like weekly-report." },
                "description": { "type": "string", "description": "When to use the skill, starting with \"Use when\"." },
                "instructions": { "type": "string", "description": "Numbered steps in Markdown." }
            },
            "required": ["name", "description", "instructions"]
        }),
    ));
    out.push(tool(
        NOTIFY,
        "Show the user a desktop notification: when a long task finishes, needs their attention, or a \
         scheduled run found something they asked to be told about. Keep it short.",
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A few words." },
                "body": { "type": "string", "description": "One or two sentences." }
            },
            "required": ["title", "body"]
        }),
    ));
    out.extend(super::sandbox::definition());
    out.extend(super::media::definition(config_dir));
    out
}

pub fn is_builtin(name: &str) -> bool {
    matches!(name, WEB_SEARCH | REMEMBER | FORGET | USE_SKILL | UPDATE_PLAN | SCHEDULE_TASK | SEARCH_LIBRARY | READ_DOCUMENT | SAVE_SKILL | KNOWLEDGE_WRITE | KNOWLEDGE_READ | KNOWLEDGE_SEARCH | SEARCH_CHATS | READ_CHAT)
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
            let recency = search::Recency::parse(&str_arg("recency"));
            let sites: Vec<String> = args.get("sites").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            let sites = search::clean_sites(&sites);
            match search::search_sites(config_dir, &query, max, recency, &sites).await {
                Ok(hits) if hits.is_empty() && !sites.is_empty() => ok(format!("No results on {} for that query. Try other words or search without sites.", sites.join(", "))),
                Ok(hits) => ok(serde_json::to_string_pretty(&json!({ "query": query, "sites": sites, "results": hits })).unwrap_or_default()),
                Err(e) => err(e),
            }
        }
        REMEMBER => {
            let mut memories = memory::load(config_dir);
            let filing = memory::Filing {
                category: str_arg("category"),
                importance: match str_arg("importance").as_str() {
                    "low" => Some(1),
                    "high" => Some(3),
                    _ => Some(2),
                },
                inferred: args.get("inferred").and_then(Value::as_bool).unwrap_or(false),
                conversation: Some(str_arg("conversation")).filter(|c| !c.is_empty()),
            };
            match memory::add_filed(&mut memories, &str_arg("fact"), "assistant", filing) {
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
        KNOWLEDGE_WRITE => {
            let sources: Vec<String> = args
                .get("sources")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            let id = str_arg("id");
            match super::knowledge::write(config_dir, (!id.is_empty()).then_some(id.as_str()), &str_arg("title"), &str_arg("content"), "assistant", &sources) {
                Ok(page) => ok(json!({ "id": page.id, "title": page.title, "saved": true }).to_string()),
                Err(e) => err(e),
            }
        }
        KNOWLEDGE_READ => {
            let pages = super::knowledge::load(config_dir);
            match super::knowledge::find(&pages, &str_arg("page")) {
                Some(page) => ok(format!("# {}\n\n{}\n\nSources: {}", page.title, page.body, if page.sources.is_empty() { "none".into() } else { page.sources.join("; ") })),
                None => err("There is no knowledge page with that id or title."),
            }
        }
        KNOWLEDGE_SEARCH => {
            let pages = super::knowledge::load(config_dir);
            let hits: Vec<Value> = super::knowledge::rank(&pages, &str_arg("query"))
                .into_iter()
                .take(5)
                .map(|(p, _)| json!({ "id": &p.id[..8], "title": p.title, "excerpt": p.body.chars().take(400).collect::<String>() }))
                .collect();
            if hits.is_empty() { ok("No knowledge page matches.") } else { ok(serde_json::to_string_pretty(&hits).unwrap_or_default()) }
        }
        READ_DOCUMENT => {
            let wanted = str_arg("document");
            let in_library = str_arg("library");
            let part = args.get("part").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            match super::libraries::read_document(config_dir, &in_library, &wanted, part) {
                Ok(text) => ok(text),
                Err(e) => err(e),
            }
        }
        SEARCH_CHATS => {
            let query = str_arg("query");
            if query.is_empty() {
                return err("Give a search query.");
            }
            let hits = search_chats(config_dir, &query);
            if hits.is_empty() {
                return ok(json!({ "query": query, "conversations": [], "note": "No earlier conversation matched; try other words." }).to_string());
            }
            let rows: Vec<Value> = hits
                .iter()
                .map(|h| json!({ "id": h.id, "title": h.title, "date": h.updated_at.format("%Y-%m-%d").to_string(), "snippet": h.snippet }))
                .collect();
            ok(serde_json::to_string_pretty(&json!({ "query": query, "conversations": rows })).unwrap_or_default())
        }
        READ_CHAT => match super::store::load_conversation(config_dir, &str_arg("id")) {
            Ok(c) if !c.temporary => ok(chat_transcript(&c, READ_CHAT_LIMIT)),
            _ => err("There is no conversation with that id; use an id from search_chats."),
        },
        SAVE_SKILL => match skills::save(config_dir, &str_arg("name"), &str_arg("description"), &str_arg("instructions")) {
            Ok(()) => ok(format!("Saved the skill `{}`. The user can call it with /{}.", str_arg("name"), str_arg("name"))),
            Err(e) => err(e),
        },
        SEARCH_LIBRARY => {
            let query = str_arg("query");
            if query.is_empty() {
                return err("Give a search query.");
            }
            let wanted = str_arg("library");
            let only: Vec<String> = if wanted.is_empty() {
                Vec::new()
            } else {
                match super::libraries::list(config_dir).into_iter().find(|l| l.name.eq_ignore_ascii_case(&wanted)) {
                    Some(l) => vec![l.id],
                    None => return err(format!("There is no library named {wanted}.")),
                }
            };
            let since_text = str_arg("since");
            let since = super::libraries::parse_since(&since_text, chrono::Utc::now());
            if !since_text.is_empty() && since.is_none() {
                return err("`since` is a date like 2026-03-01, or a span like 30d, 6m or 1y.");
            }
            // With a cutoff, a wider pool is searched and the older documents' passages left out.
            let (hits, note) = super::embeddings::search(config_dir, &only, &query, if since.is_some() { 30 } else { 6 }).await;
            let hits: Vec<_> = match since {
                Some(cutoff) => super::libraries::changed_since(config_dir, hits, cutoff).into_iter().take(6).collect(),
                None => hits,
            };
            let note = match (since, hits.is_empty()) {
                (Some(cutoff), true) => Some(format!("No document changed since {} matched; search without `since` to include older ones.", cutoff.format("%Y-%m-%d"))),
                _ => note,
            };
            if hits.is_empty() {
                ok(json!({ "query": query, "passages": [], "note": note.unwrap_or_else(|| "Nothing matched; try other keywords.".into()) }).to_string())
            } else {
                ok(serde_json::to_string_pretty(&json!({ "query": query, "passages": hits, "note": note })).unwrap_or_default())
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
        "yearly" => Frequency::Yearly {
            month: args.get("month").and_then(Value::as_u64).unwrap_or(1).clamp(1, 12) as u8,
            day: args.get("day_of_month").and_then(Value::as_u64).unwrap_or(1).clamp(1, 31) as u8,
            time,
        },
        other => return err(format!("Unknown frequency `{other}`.")),
    };
    let agent_id = match s("agent") {
        name if name.is_empty() => None,
        name => match super::agents::load(config_dir).into_iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
            Some(agent) => Some(agent.id),
            None => return err(format!("There is no agent named {name}.")),
        },
    };
    // The engine fills `reply_in` with this conversation's id when the model asked for it.
    let reply_in = args.get("reply_in").and_then(Value::as_str).filter(|_| args.get("in_this_chat").and_then(Value::as_bool) == Some(true)).map(str::to_string);
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
        seen: None,
        reply_in,
        webhook: None,
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

/// How much of an earlier conversation `read_chat` returns.
const READ_CHAT_LIMIT: usize = 24_000;

/// Earlier conversations matching `query`: every word first; failing that, the most words.
pub(crate) fn search_chats(config_dir: &Path, query: &str) -> Vec<super::index::Hit> {
    let all = super::index::search(config_dir, query, 8);
    let words: Vec<&str> = query.split_whitespace().filter(|w| w.chars().count() > 2).collect();
    if !all.is_empty() || words.len() < 2 {
        return all;
    }
    let mut found: Vec<(usize, super::index::Hit)> = Vec::new();
    for word in words {
        for hit in super::index::search(config_dir, word, 20) {
            match found.iter_mut().find(|(_, h)| h.id == hit.id) {
                Some((n, _)) => *n += 1,
                None => found.push((1, hit)),
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.updated_at.cmp(&a.1.updated_at)));
    found.into_iter().take(8).map(|(_, h)| h).collect()
}

/// What was said in a conversation, as plain text, cut at `limit` characters.
pub(super) fn chat_transcript(conversation: &super::store::Conversation, limit: usize) -> String {
    let mut text = format!("# {}
Date: {}
\n", conversation.title, conversation.created_at.format("%Y-%m-%d"));
    for turn in super::store::turns(conversation) {
        let said: Vec<String> = turn
            .parts
            .iter()
            .filter_map(|p| match p {
                super::store::Part::Text { text } => Some(text.clone()),
                super::store::Part::Tool { name, .. } => Some(format!("[used {name}]")),
                _ => None,
            })
            .collect();
        if said.is_empty() {
            continue;
        }
        text.push_str(if turn.role == "user" { "User: " } else { "Assistant: " });
        text.push_str(&said.join("\n"));
        text.push_str("\n\n");
        if text.chars().count() > limit {
            let cut: String = text.chars().take(limit).collect();
            return format!("{cut}\n\n[The rest of the conversation is cut off.]");
        }
    }
    text
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
        // A follow-up in the chat it was asked in; `reply_in` is set by the engine, and only used
        // when the model asked for it.
        run(dir.path(), SCHEDULE_TASK, &json!({"title": "Check prices", "prompt": "Check again", "frequency": "once", "date": "2030-01-02", "time": "08:00", "in_this_chat": true, "reply_in": "conv-1"})).await;
        run(dir.path(), SCHEDULE_TASK, &json!({"title": "Other", "prompt": "New chat", "frequency": "daily", "time": "08:00", "reply_in": "conv-1"})).await;
        let tasks = super::super::tasks::load(dir.path());
        assert_eq!(tasks.iter().find(|t| t.title == "Check prices").unwrap().reply_in.as_deref(), Some("conv-1"));
        assert_eq!(tasks.iter().find(|t| t.title == "Other").unwrap().reply_in, None);
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
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).filter(|n| n != super::super::sandbox::RUN_CODE && n != super::super::media::GENERATE_IMAGE && n != SAVE_SKILL && n != NOTIFY && !n.starts_with("knowledge_")).collect();
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, SCHEDULE_TASK, SEARCH_CHATS, READ_CHAT, REMEMBER, FORGET, USE_SKILL]);
        memory::save(dir.path(), &memory::Memories { enabled: false, items: vec![] }).unwrap();
        let names: Vec<String> = definitions(dir.path()).into_iter().map(|t| t.name).filter(|n| n != super::super::sandbox::RUN_CODE && n != super::super::media::GENERATE_IMAGE && n != SAVE_SKILL && n != NOTIFY && !n.starts_with("knowledge_")).collect();
        assert_eq!(names, [WEB_SEARCH, UPDATE_PLAN, DELEGATE, CANVAS, SCHEDULE_TASK, USE_SKILL]);
    }

    #[tokio::test]
    async fn earlier_chats_are_found_and_read() {
        use rig_core::completion::Message;
        let dir = tempfile::tempdir().unwrap();
        let mut c = super::super::store::Conversation::new("c1".into(), None, None);
        c.title = "Kitchen renovation".into();
        c.messages = vec![Message::user("Which oak worktop should we pick?"), Message::assistant("The 40 mm solid oak one, oiled.")];
        super::super::store::save_conversation(dir.path(), &c).unwrap();
        let found = run(dir.path(), SEARCH_CHATS, &json!({"query": "oak worktop"})).await;
        assert!(found.text.contains("\"c1\"") && found.text.contains("Kitchen renovation"), "{}", found.text);
        // Not every word needs to match.
        assert!(run(dir.path(), SEARCH_CHATS, &json!({"query": "walnut worktop"})).await.text.contains("c1"));
        let read = run(dir.path(), READ_CHAT, &json!({"id": "c1"})).await;
        assert!(read.text.contains("User: Which oak") && read.text.contains("Assistant: The 40 mm"));
        assert!(run(dir.path(), READ_CHAT, &json!({"id": "nope"})).await.is_error);
    }
}
