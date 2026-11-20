//! What the model is told about each built-in tool: its name, description and JSON Schema.

use super::*;

pub(super) fn tool(name: &str, description: &str, parameters: Value) -> ChatTool {
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
pub(in crate::chat::turn) fn definitions(
    bench: &crate::chat::workbench::Bench,
    config_dir: &Path,
) -> Vec<ChatTool> {
    let mut out = Vec::new();
    work_tools(&mut out);
    source_tools(config_dir, &mut out);
    memory_tools(config_dir, &mut out);
    other_tools(bench, config_dir, &mut out);
    review_tools(&mut out);
    out
}

/// Working on the request: web search, the plan, sub-agents, canvases and scheduled tasks.
fn work_tools(out: &mut Vec<ChatTool>) {
    out.push(tool(
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
    ));
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
}

/// The user's own material, offered when there is some: libraries, calendars and contacts.
fn source_tools(config_dir: &Path, out: &mut Vec<ChatTool>) {
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
}

/// Memory and knowledge pages while memory is on, and skills while any is enabled.
fn memory_tools(config_dir: &Path, out: &mut Vec<ChatTool>) {
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
             not save it. When the user asks you to remember something, this call is how it is \
             kept: make one per fact.",
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
}

/// Saving skills, notifications, other agents, code, web pages, media and animation.
fn other_tools(bench: &crate::chat::workbench::Bench, config_dir: &Path, out: &mut Vec<ChatTool>) {
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
    out.extend(crate::chat::sandbox::definition(bench, config_dir));
    out.extend(crate::chat::webpage::definition());
    out.extend(crate::chat::webpage::act_definition());
    out.extend(crate::chat::media::definition(config_dir));
    out.extend(crate::chat::videogen::definition(config_dir));
    out.push(crate::chat::animation::definition());
}

/// Looking back at its own runs: traces, a run, usage and failures.
fn review_tools(out: &mut Vec<ChatTool>) {
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
}
