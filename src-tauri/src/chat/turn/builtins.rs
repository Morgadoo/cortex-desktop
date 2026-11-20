//! Tools the app itself provides, beside the connectors: web search, memory and skills.

use std::path::Path;

use serde_json::{json, Value};

use crate::chat::tools::{ChatTool, Outcome};
use crate::chat::{memory, search, skills};

pub use crate::chat::tool_names::*;

/// What a built-in can change, which decides how it meets the approval mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, searches or shows something, and changes nothing.
    Reads,
    /// Writes the assistant's own memory, which the user can switch off and edit in Settings.
    /// It never asks, so the assistant can keep notes as it goes.
    Memory,
    /// Leaves something behind that acts later or changes how the assistant works: a scheduled
    /// task, a saved skill. It follows the approval mode like a connector tool that changes
    /// something.
    Standing,
}

pub fn effect(name: &str) -> Effect {
    match name {
        REMEMBER | FORGET | KNOWLEDGE_WRITE => Effect::Memory,
        SCHEDULE_TASK | SAVE_SKILL => Effect::Standing,
        _ => Effect::Reads,
    }
}

fn tool(name: &str, description: &str, parameters: Value) -> ChatTool {
    ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only: effect(name) == Effect::Reads,
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
                    "maxItems": crate::chat::subagents::MAX_TASKS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "A few words." },
                            "instructions": { "type": "string", "description": "What to find or check, and what to report." },
                            "agent": { "type": "string", "description": "Optional: the name of an agent from your team to do this task." },
                            "tools": { "type": "array", "items": { "type": "string" }, "description": "Optional: only these tools for this task (web_search, use_skill, browser__fetch…); all reading tools when left out." }
                        },
                        "required": ["title", "instructions"]
                    }
                },
                "mode": {
                    "type": "string",
                    "enum": ["parallel", "sequential", "debate"],
                    "description": "parallel (default): all at once. sequential: one after another, each seeing the earlier reports. debate: all at once, then a second round where each answers the others."
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
                "content": { "type": "string", "description": "The full document, in Markdown (or the app's HTML)." },
                "files": {
                    "type": "array",
                    "description": "For a bigger app instead of `content`: its files, index.html first, linked with relative paths (style.css, js/app.js). Send every file each time.",
                    "items": { "type": "object", "properties": { "path": { "type": "string" }, "content": { "type": "string" } }, "required": ["path", "content"] }
                }
            }
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
                "frequency": { "type": "string", "enum": ["once", "daily", "weekly", "monthly", "yearly", "cron"] },
                "time": { "type": "string", "description": "24-hour local time, HH:MM (not used with cron)." },
                "cron": { "type": "string", "description": "For frequency cron: five fields, minute hour day-of-month month day-of-week, local time; e.g. \"*/30 9-17 * * 1-5\" is every half hour in working hours." },
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
    let libraries = crate::chat::libraries::list(config_dir);
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
    let sources = crate::chat::calendar::load(config_dir);
    if !sources.calendars.is_empty() {
        let names: Vec<&str> = sources.calendars.iter().map(|c| c.name.as_str()).collect();
        out.push(tool(
            CALENDAR_EVENTS,
            &format!(
                "Read the user's calendars ({}): the events between two dates, in their local time, \
                 repeating events included. Use it for their schedule, free time, deadlines and \
                 meetings. Read-only.",
                names.join(", ")
            ),
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "First day, YYYY-MM-DD. Default: today." },
                    "to": { "type": "string", "description": "Last day, YYYY-MM-DD. Default: a week after `from`." },
                    "query": { "type": "string", "description": "Only events whose title, place or notes contain this." }
                }
            }),
        ));
    }
    if !sources.contacts.is_empty() {
        out.push(tool(
            FIND_CONTACT,
            "Look up people in the user's contacts by name, email, company or role: their emails, \
             phones, organization, birthday and address. Read-only.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "A name, email, company or role." } },
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
    let memories = memory::load(config_dir);
    if memories.enabled {
        // "Save new facts" off: knowledge is used but not changed.
        if memories.update {
            out.push(tool(
            KNOWLEDGE_WRITE,
            "Create or update a knowledge page: lasting, detailed context worth keeping across \
             conversations (a project, a client, a process, how the user wants something done). \
             Write only what will still be true and relevant in a month, in another conversation. \
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
        }
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
            MEMORY_SEARCH,
            "Search the facts saved about the user, including those not listed in your \
             instructions.",
            json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }),
        ));
        if memories.update {
            out.push(tool(
            REMEMBER,
            "Save one lasting fact about the user for future conversations. Before saving, ask: \
             will this still be true and useful in a month, in another conversation? If not, do \
             not save it.",
            json!({
                "type": "object",
                "properties": {
                    "fact": { "type": "string", "description": "A short, self-contained fact." },
                    "category": { "type": "string", "enum": ["preference", "work", "person", "project", "habit", "fact"], "description": "What it is about." },
                    "importance": { "type": "string", "enum": ["low", "normal", "high"], "description": "high: keep it in mind always (a strong preference, a constraint); low: a passing detail." },
                    "inferred": { "type": "boolean", "description": "true when you concluded it rather than the user saying it." },
                    "scope": { "type": "string", "enum": ["everywhere", "project", "agent"], "description": "Where it applies: everywhere (the default), only in this project, or only for this agent." }
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
    }
    if skills::list(config_dir).iter().any(|s| s.enabled) {
        out.push(tool(
            USE_SKILL,
            "Load a skill's instructions by name, then follow them. With `file`, read one of the \
             skill's supporting files instead (they are listed when the skill loads).",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "file": { "type": "string", "description": "A supporting file's path, as listed with the skill." }
                },
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
    if !crate::chat::agents::load(config_dir).is_empty() {
        out.push(tool(
            crate::chat::agents::MESSAGE_AGENT,
            "Write to one of the user's agents by name and get its reply. It works on the message in a chat \
             of its own, with its own instructions, tools and memory; the user can open that chat. Share files \
             with `files` (paths in this chat's folder, or full paths). Use it when another agent owns the \
             subject; use delegate for quick parallel research.",
            json!({
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "The agent's name." },
                    "message": { "type": "string", "description": "What you need, standing alone: the agent sees nothing of this chat." },
                    "files": { "type": "array", "items": { "type": "string" }, "maxItems": 5, "description": "Text files to share." }
                },
                "required": ["agent", "message"]
            }),
        ));
    }
    out.extend(crate::chat::sandbox::definition(config_dir));
    out.extend(crate::chat::webpage::definition());
    out.extend(crate::chat::webpage::act_definition());
    out.extend(crate::chat::media::definition(config_dir));
    out.extend(crate::chat::videogen::definition(config_dir));
    out.push(crate::chat::animation::definition());
    let days = json!({
        "type": "object",
        "properties": {
            "days": { "type": "integer", "minimum": 1, "maximum": 365, "default": 7, "description": "How many days back." }
        }
    });
    out.push(tool(
        QUERY_TRACES,
        "List your recent runs, newest first: when, which model, how they ended, how many tool calls \
         failed, what they cost. Filter by outcome (done, cancelled, error), by a tool they called, \
         or to runs where a tool failed. Measurements only; open the chat with read_chat to see what \
         was said.",
        json!({
            "type": "object",
            "properties": {
                "days": { "type": "integer", "minimum": 1, "maximum": 365, "default": 7 },
                "outcome": { "type": "string", "enum": ["done", "cancelled", "error"] },
                "tool": { "type": "string" },
                "failed_tools": { "type": "boolean" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20 }
            }
        }),
    ));
    out.push(tool(
        GET_RUN,
        "Open one run from query_traces: each model call (time, tokens, errors), each tool call \
         (time, success), and which skills it had in front of it and which it used.",
        json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
    ));
    out.push(tool(
        USAGE_STATS,
        "Your own recent activity, measured: runs, cost, and per tool and model how often each was          used, how often it failed and how long it took. No messages or results, only numbers. Use it          when the user asks what you have been doing or spending, or to see which tools work. \
         With `artifact` (kind:id, e.g. skill:summarize, script_tool:count_words, tool:web_search), \
         that one thing: how often it was offered and used, and its score.",
        json!({
            "type": "object",
            "properties": {
                "days": { "type": "integer", "minimum": 1, "maximum": 365, "default": 7, "description": "How many days back." },
                "artifact": { "type": "string", "description": "kind:id of one skill, tool, page or fact." }
            }
        }),
    ));
    out.push(tool(
        FAILURE_SUMMARY,
        "What failed in your recent runs, most often first: tools that returned errors and models          that failed. Use it to find out why things went wrong before proposing a fix.",
        days,
    ));
    out
}

fn ok(text: impl Into<String>) -> Outcome {
    Outcome::ok(text)
}

fn err(text: impl Into<String>) -> Outcome {
    Outcome::err(text)
}

/// A string argument, trimmed; empty when it is missing.
fn text_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Runs the built-in `name`: one function per tool below.
pub async fn run(config_dir: &Path, name: &str, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    if matches!(name, REMEMBER | FORGET | KNOWLEDGE_WRITE) && !memory::load(config_dir).update {
        return err("The user has turned off saving to knowledge; nothing was changed.");
    }
    match name {
        WEB_SEARCH => web_search(config_dir, args).await,
        REMEMBER => remember(config_dir, args).await,
        FORGET => forget(config_dir, args).await,
        SCHEDULE_TASK => schedule(config_dir, args),
        KNOWLEDGE_WRITE => knowledge_write(config_dir, args).await,
        KNOWLEDGE_READ => knowledge_read(config_dir, args).await,
        KNOWLEDGE_SEARCH => knowledge_search(config_dir, args).await,
        READ_DOCUMENT => read_document(config_dir, args).await,
        SEARCH_CHATS => search_chats(config_dir, args).await,
        MEMORY_SEARCH => memory_search(config_dir, args).await,
        QUERY_TRACES => query_traces(config_dir, args).await,
        GET_RUN => get_run(config_dir, args).await,
        USAGE_STATS if !str_arg("artifact").is_empty() => {
            usage_stats_of_artifact(config_dir, args).await
        }
        USAGE_STATS | FAILURE_SUMMARY => usage_or_failures(config_dir, args, name).await,
        READ_CHAT => read_chat(config_dir, args).await,
        SAVE_SKILL => save_skill(config_dir, args).await,
        CALENDAR_EVENTS => calendar_events(config_dir, args).await,
        FIND_CONTACT => find_contact(config_dir, args).await,
        SEARCH_LIBRARY => search_library(config_dir, args).await,
        UPDATE_PLAN => update_plan(config_dir, args).await,
        USE_SKILL => use_skill(config_dir, args).await,
        _ => err(format!("Unknown built-in tool {name}.")),
    }
}

/// `WEB_SEARCH`.
async fn web_search(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a search query.");
    }
    let max = args
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(6)
        .clamp(1, 10) as usize;
    let recency = search::Recency::parse(&str_arg("recency"));
    let sites: Vec<String> = args
        .get("sites")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let sites = search::clean_sites(&sites);
    match search::search_sites(config_dir, &query, max, recency, &sites).await {
        Ok(hits) if hits.is_empty() && !sites.is_empty() => ok(format!(
            "No results on {} for that query. Try other words or search without sites.",
            sites.join(", ")
        )),
        Ok(hits) => ok(serde_json::to_string_pretty(
            &json!({ "query": query, "sites": sites, "results": hits }),
        )
        .unwrap_or_default()),
        Err(e) => err(e),
    }
}

/// `REMEMBER`.
async fn remember(config_dir: &Path, args: &Value) -> Outcome {
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
async fn forget(config_dir: &Path, args: &Value) -> Outcome {
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
async fn knowledge_write(config_dir: &Path, args: &Value) -> Outcome {
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
async fn knowledge_read(config_dir: &Path, args: &Value) -> Outcome {
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
async fn knowledge_search(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let pages = crate::chat::knowledge::load(config_dir);
    let hits: Vec<Value> = crate::chat::knowledge::rank(&pages, &str_arg("query"))
            .into_iter()
            .take(5)
            .map(|(p, _)| json!({ "id": &p.id[..8], "title": p.title, "excerpt": p.body.chars().take(400).collect::<String>() }))
            .collect();
    if hits.is_empty() {
        ok("No knowledge page matches.")
    } else {
        ok(serde_json::to_string_pretty(&hits).unwrap_or_default())
    }
}

/// `READ_DOCUMENT`.
async fn read_document(config_dir: &Path, args: &Value) -> Outcome {
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
async fn search_chats(config_dir: &Path, args: &Value) -> Outcome {
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
async fn memory_search(config_dir: &Path, args: &Value) -> Outcome {
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

/// `QUERY_TRACES`.
async fn query_traces(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let days = args
        .get("days")
        .and_then(Value::as_i64)
        .unwrap_or(7)
        .clamp(1, 365);
    let filter = crate::chat::traces::RunFilter {
        since: Some(chrono::Utc::now() - chrono::Duration::days(days)),
        outcome: Some(str_arg("outcome")).filter(|o| !o.is_empty()),
        tool: Some(str_arg("tool")).filter(|t| !t.is_empty()),
        failed_tools: args
            .get("failed_tools")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limit: args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
    };
    ok(
        serde_json::to_string_pretty(&crate::chat::traces::runs(config_dir, &filter))
            .unwrap_or_default(),
    )
}

/// `GET_RUN`.
async fn get_run(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match crate::chat::traces::run_detail(config_dir, &str_arg("id")) {
        Some(detail) => ok(serde_json::to_string_pretty(&detail).unwrap_or_default()),
        None => err("There is no run with that id; list them with query_traces."),
    }
}

/// `USAGE_STATS`.
async fn usage_stats_of_artifact(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    // One artifact: how often it was in front of the model and used, and its score.
    let wanted = str_arg("artifact");
    let Some((kind, id)) = wanted.split_once(':') else {
        return err("Name the artifact as kind:id, e.g. skill:summarize or tool:web_search.");
    };
    match crate::chat::traces::artifact(config_dir, kind, id) {
        Some(a) => ok(serde_json::to_string_pretty(&json!({
            "artifact": wanted,
            "status": a.status,
            "version": a.version,
            "loaded": a.loaded,
            "used": a.used,
            "lastUsed": a.last_used,
            "critical": a.critical,
            "score": cortex_kernel::lifecycle::score(&a, chrono::Utc::now()),
        }))
        .unwrap_or_default()),
        None => err("Nothing by that name has been recorded."),
    }
}

/// `USAGE_STATS | FAILURE_SUMMARY`.
async fn usage_or_failures(config_dir: &Path, args: &Value, name: &str) -> Outcome {
    let days = args.get("days").and_then(Value::as_i64).unwrap_or(7);
    ok(if name == USAGE_STATS {
        crate::chat::traces::usage_report(config_dir, days)
    } else {
        crate::chat::traces::failure_report(config_dir, days)
    })
}

/// `READ_CHAT`.
async fn read_chat(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match crate::chat::store::load_conversation(config_dir, &str_arg("id")) {
        Ok(c) if !c.temporary => ok(crate::chat::store::chat_transcript(&c, READ_CHAT_LIMIT)),
        _ => err("There is no conversation with that id; use an id from search_chats."),
    }
}

/// `SAVE_SKILL`.
async fn save_skill(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let provenance = skill_provenance(config_dir, &str_arg("name"), &str_arg("conversation"));
    match skills::save(
        config_dir,
        &str_arg("name"),
        &str_arg("description"),
        &str_arg("instructions"),
    ) {
        Ok(()) => {
            crate::chat::traces::register(
                config_dir,
                "skill",
                &str_arg("name"),
                Some(&crate::chat::traces::hash(&str_arg("instructions"))),
                Some(&provenance),
                "active",
            );
            ok(format!(
                "Saved the skill `{}`. The user can call it with /{}.",
                str_arg("name"),
                str_arg("name")
            ))
        }
        Err(e) => err(e),
    }
}

/// `CALENDAR_EVENTS`.
async fn calendar_events(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let today = chrono::Local::now().date_naive();
    let date = |key: &str| chrono::NaiveDate::parse_from_str(&str_arg(key), "%Y-%m-%d").ok();
    let from_day = date("from").unwrap_or(today);
    let to_day = date("to")
        .unwrap_or(from_day + chrono::Duration::days(7))
        .max(from_day);
    if (to_day - from_day).num_days() > 366 {
        return err("Ask for at most a year at a time.");
    }
    let local = |d: chrono::NaiveDate, end: bool| {
        use chrono::TimeZone as _;
        let t = if end {
            d.and_hms_opt(23, 59, 59)
        } else {
            d.and_hms_opt(0, 0, 0)
        }
        .unwrap_or_default();
        chrono::Local
            .from_local_datetime(&t)
            .earliest()
            .map_or_else(chrono::Utc::now, |t| t.with_timezone(&chrono::Utc))
    };
    let (from, to) = (local(from_day, false), local(to_day, true));
    let mut events = Vec::new();
    let mut problems = Vec::new();
    for cal in crate::chat::calendar::load(config_dir).calendars {
        match crate::chat::calendar::fetch(&cal.location).await {
            Ok(text) => events.extend(crate::chat::calendar::occurrences(
                &cal.name,
                &crate::chat::calendar::parse_ics(&text),
                from,
                to,
                &str_arg("query"),
            )),
            Err(e) => problems.push(format!("{}: {e}", cal.name)),
        }
    }
    events.sort_by_key(|e| e.at);
    let shown: Vec<_> = events.iter().take(80).collect();
    ok(serde_json::to_string_pretty(&json!({
        "from": from_day.to_string(),
        "to": to_day.to_string(),
        "events": shown,
        "more": events.len().saturating_sub(shown.len()),
        "unreadable": problems,
    }))
    .unwrap_or_default())
}

/// `FIND_CONTACT`.
async fn find_contact(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a name, email or company to look up.");
    }
    let mut all = Vec::new();
    let mut problems = Vec::new();
    for book in crate::chat::calendar::load(config_dir).contacts {
        match crate::chat::calendar::fetch(&book.location).await {
            Ok(text) => all.extend(crate::chat::calendar::parse_vcards(&text)),
            Err(e) => problems.push(format!("{}: {e}", book.name)),
        }
    }
    let found = crate::chat::calendar::find(&all, &query);
    ok(serde_json::to_string_pretty(
        &json!({ "query": query, "contacts": found, "unreadable": problems }),
    )
    .unwrap_or_default())
}

/// `SEARCH_LIBRARY`.
async fn search_library(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a search query.");
    }
    let wanted = str_arg("library");
    let only: Vec<String> = if wanted.is_empty() {
        Vec::new()
    } else {
        match crate::chat::libraries::list(config_dir)
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case(&wanted))
        {
            Some(l) => vec![l.id],
            None => return err(format!("There is no library named {wanted}.")),
        }
    };
    let since_text = str_arg("since");
    let since = crate::chat::libraries::parse_since(&since_text, chrono::Utc::now());
    if !since_text.is_empty() && since.is_none() {
        return err("`since` is a date like 2026-03-01, or a span like 30d, 6m or 1y.");
    }
    // With a cutoff, a wider pool is searched and the older documents' passages left out.
    let (hits, note) = crate::chat::embeddings::search(
        config_dir,
        &only,
        &query,
        if since.is_some() { 30 } else { 6 },
    )
    .await;
    let hits: Vec<_> = match since {
        Some(cutoff) => crate::chat::libraries::changed_since(config_dir, hits, cutoff)
            .into_iter()
            .take(6)
            .collect(),
        None => hits,
    };
    let note = match (since, hits.is_empty()) {
        (Some(cutoff), true) => Some(format!(
            "No document changed since {} matched; search without `since` to include older ones.",
            cutoff.format("%Y-%m-%d")
        )),
        _ => note,
    };
    if hits.is_empty() {
        ok(json!({ "query": query, "passages": [], "note": note.unwrap_or_else(|| "Nothing matched; try other keywords.".into()) }).to_string())
    } else {
        ok(
            serde_json::to_string_pretty(
                &json!({ "query": query, "passages": hits, "note": note }),
            )
            .unwrap_or_default(),
        )
    }
}

/// `UPDATE_PLAN`.
async fn update_plan(_config_dir: &Path, args: &Value) -> Outcome {
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if steps == 0 {
        err("Give the plan's steps.")
    } else {
        ok(format!("Plan shown to the user ({steps} steps)."))
    }
}

/// `USE_SKILL`.
async fn use_skill(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match skills::find(config_dir, &str_arg("name"))
        .inspect(|skill| crate::chat::traces::used(config_dir, "skill", &skill.name))
    {
        Some(skill) if skill.enabled && !str_arg("file").trim().is_empty() => {
            match skills::read_file(config_dir, &skill.name, str_arg("file").trim()) {
                Ok(text) => ok(text),
                Err(e) => err(format!(
                    "{e} Its files: {}.",
                    if skill.files.is_empty() {
                        "none".to_string()
                    } else {
                        skill.files.join(", ")
                    }
                )),
            }
        }
        Some(skill) if skill.enabled => ok(skills::loaded(&skill)),
        Some(_) => err("That skill is turned off."),
        None => {
            // Not a name: search the skills with the words given.
            let all = skills::list(config_dir);
            let enabled: Vec<&skills::Skill> = all.iter().filter(|s| s.enabled).collect();
            let found: Vec<String> = skills::relevant(&enabled, &str_arg("name"))
                .into_iter()
                .take(5)
                .map(|s| format!("- {}: {}", s.name, s.description))
                .collect();
            if found.is_empty() {
                err("There is no skill with that name, and none matches those words.")
            } else {
                ok(format!(
                    "No skill has that exact name. These match; call use_skill again with one \
                     of their names:\n{}",
                    found.join("\n")
                ))
            }
        }
    }
}

/// Who a skill counts as made by, for the lifecycle: a skill the user wrote stays theirs when the
/// assistant updates it, so it is never retired as the assistant's; a new one is the assistant's,
/// with the chat it came from.
pub(crate) fn skill_provenance(config_dir: &Path, name: &str, conversation: &str) -> String {
    let users = skills::find(config_dir, name).is_some_and(|s| s.origin == "user")
        && crate::chat::traces::artifact(config_dir, "skill", name)
            .is_none_or(|a| a.provenance.as_deref().is_none_or(|p| p == "user"));
    if users {
        "user".into()
    } else if conversation.is_empty() {
        "assistant".into()
    } else {
        conversation.into()
    }
}

fn schedule(config_dir: &Path, args: &Value) -> Outcome {
    use crate::chat::tasks::{self, Frequency};
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let time = s("time");
    let frequency = match s("frequency").as_str() {
        "once" => Frequency::Once {
            date: s("date"),
            time,
        },
        "daily" => Frequency::Daily { time },
        "weekly" => {
            let days: Vec<u8> = args
                .get("days")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(tasks::weekday_index)
                        .collect()
                })
                .unwrap_or_default();
            Frequency::Weekly { days, time }
        }
        "monthly" => Frequency::Monthly {
            day: args
                .get("day_of_month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .min(31) as u8,
            time,
        },
        "yearly" => Frequency::Yearly {
            month: args
                .get("month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 12) as u8,
            day: args
                .get("day_of_month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 31) as u8,
            time,
        },
        "cron" => Frequency::Cron {
            expression: s("cron"),
        },
        other => return err(format!("Unknown frequency `{other}`.")),
    };
    let agent_id = match s("agent") {
        name if name.is_empty() => None,
        name => match crate::chat::agents::load(config_dir)
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
        {
            Some(agent) => Some(agent.id),
            None => return err(format!("There is no agent named {name}.")),
        },
    };
    // The engine fills `reply_in` with this conversation's id when the model asked for it.
    let reply_in = args
        .get("reply_in")
        .and_then(Value::as_str)
        .filter(|_| args.get("in_this_chat").and_then(Value::as_bool) == Some(true))
        .map(str::to_string);
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
        comments: Vec::new(),
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

#[cfg(test)]
mod tests;
