//! The MCP endpoint (`/mcp`): the assistant as tools for other MCP clients.

use super::*;

/// The MCP protocol version this server speaks when a client names none it knows.
pub(super) const MCP_VERSION: &str = "2025-06-18";

/// The tools Cortex offers MCP clients.
pub(in crate::chat::integrations) fn mcp_tools() -> Value {
    json!([
        {
            "name": "search_libraries",
            "description": "Search the user's document libraries in Cortex and get the most relevant passages with their document names.",
            "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "library": { "type": "string", "description": "A library name, to search only that one." } }, "required": ["query"] }
        },
        {
            "name": "ask",
            "description": "Ask one of the user's models or agents in Cortex and get its answer. `model` is an id from list_models (`vendor/model` or `agent/<id>`); the default model when left out.",
            "inputSchema": { "type": "object", "properties": { "prompt": { "type": "string" }, "model": { "type": "string" } }, "required": ["prompt"] }
        },
        {
            "name": "list_models",
            "description": "The models and agents Cortex can answer with.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_tasks",
            "description": "The user's scheduled tasks in Cortex.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "run_task",
            "description": "Start one of the user's scheduled tasks now, with optional input added to its prompt; its result arrives in Cortex as a conversation.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string" }, "input": { "type": "string" } }, "required": ["id"] }
        }
    ])
}

pub(super) fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(super) fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub(super) fn tool_text(text: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": is_error })
}

pub(super) async fn mcp_no_stream(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &ctx.token) {
        return error(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.");
    }
    // No server-initiated messages: every answer comes back on its POST.
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "This MCP server answers each POST directly; it opens no event stream.",
    )
        .into_response()
}

pub(super) async fn mcp(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = std::time::Instant::now();
    if !authorized(&headers, &ctx.token) {
        record("mcp", "", StatusCode::UNAUTHORIZED, started);
        return error(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.");
    }
    let Ok(Json(message)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(&Value::Null, -32700, "Parse error")),
        )
            .into_response();
    };
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let response = match mcp_answer(&ctx, &message).await {
        Some(answer) => Json(answer).into_response(),
        // Notifications get no answer.
        None => StatusCode::ACCEPTED.into_response(),
    };
    record(
        &format!("mcp {method}"),
        message
            .pointer("/params/arguments/model")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        response.status(),
        started,
    );
    response
}

/// The answer to one JSON-RPC message, or `None` for a notification.
pub(super) async fn mcp_answer(ctx: &Ctx, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(MCP_VERSION);
            let version = if ["2025-06-18", "2025-03-26", "2024-11-05"].contains(&asked) {
                asked
            } else {
                MCP_VERSION
            };
            rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "cortex", "title": "Cortex", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Cortex is the user's desktop assistant. Search their document libraries, ask their models or agents, and start their scheduled tasks."
                }),
            )
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, json!({ "tools": mcp_tools() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            rpc_result(&id, mcp_call(ctx, name, &args).await)
        }
        _ => rpc_error(&id, -32601, &format!("Method not found: {method}")),
    })
}

pub(super) async fn mcp_call(ctx: &Ctx, name: &str, args: &Value) -> Value {
    let text = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    match name {
        "search_libraries" => {
            let query = text("query");
            if query.is_empty() {
                return tool_text("Give a query.", true);
            }
            let only: Vec<String> = match text("library") {
                l if l.is_empty() => Vec::new(),
                l => crate::chat::libraries::list(&ctx.config_dir)
                    .into_iter()
                    .filter(|x| x.name.eq_ignore_ascii_case(&l))
                    .map(|x| x.id)
                    .collect(),
            };
            let (passages, note) =
                crate::chat::embeddings::search(&ctx.config_dir, &only, &query, 8).await;
            if passages.is_empty() {
                return tool_text("No passage matched.", false);
            }
            let mut out: Vec<String> = passages
                .iter()
                .map(|p| {
                    format!(
                        "[{} — {}, passage {}]\n{}",
                        p.document,
                        p.library,
                        p.passage + 1,
                        p.text
                    )
                })
                .collect();
            if let Some(note) = note {
                out.insert(0, note);
            }
            tool_text(out.join("\n\n"), false)
        }
        "ask" => {
            let prompt = text("prompt");
            if prompt.is_empty() {
                return tool_text("Give a prompt.", true);
            }
            let model = match text("model") {
                m if m.is_empty() => match crate::chat::providers::load(&ctx.config_dir)
                    .default_model
                {
                    Some(m) => m,
                    None => {
                        return tool_text("No default model is set in Cortex; pass `model`.", true)
                    }
                },
                m => m,
            };
            let (connected, agent) = match resolve(&ctx.config_dir, &model).await {
                Ok(m) => m,
                Err(e) => return tool_text(e, true),
            };
            match connected
                .call(with_agent(
                    CompletionRequest::new(Message::user(prompt)),
                    agent,
                ))
                .await
            {
                Ok(response) => tool_text(response.text(), false),
                Err(e) => tool_text(crate::chat::context::short_error(&e.to_string()), true),
            }
        }
        "list_models" => {
            let file = crate::chat::providers::load(&ctx.config_dir);
            let mut lines: Vec<String> = crate::chat::providers::models(&file)
                .await
                .into_iter()
                .map(|m| format!("{} — {}", m.reference, m.name))
                .collect();
            lines.extend(
                crate::chat::agents::load(&ctx.config_dir)
                    .into_iter()
                    .map(|a| format!("{AGENT_PREFIX}{} — agent {}", a.id, a.name)),
            );
            tool_text(lines.join("\n"), false)
        }
        "list_tasks" => {
            let lines: Vec<String> = crate::chat::tasks::load(&ctx.config_dir)
                .into_iter()
                .map(|t| {
                    format!(
                        "{} — {} ({}{})",
                        t.id,
                        t.title,
                        crate::chat::tasks::describe(&t.frequency),
                        if t.enabled { "" } else { ", paused" }
                    )
                })
                .collect();
            tool_text(
                if lines.is_empty() {
                    "No scheduled tasks.".to_string()
                } else {
                    lines.join("\n")
                },
                false,
            )
        }
        "run_task" => {
            let Some(mut task) = crate::chat::tasks::load(&ctx.config_dir)
                .into_iter()
                .find(|t| t.id == text("id"))
            else {
                return tool_text("No task with that id; see list_tasks.", true);
            };
            let input: String = text("input").chars().take(20_000).collect();
            if !input.is_empty() {
                task.prompt = format!(
                    "{}\n\nThis run was started from outside the app with this input:\n{input}",
                    task.prompt
                );
            }
            crate::chat::tasks::record(
                &ctx.config_dir,
                &task.id,
                task.last_conversation.as_deref().unwrap_or_default(),
                crate::chat::tasks::TaskStatus::Running,
            );
            let title = task.title.clone();
            let app = ctx.app.clone();
            tauri::async_runtime::spawn(
                async move { crate::chat::engine::run_task(app, task).await },
            );
            tool_text(
                format!("Started “{title}”. Its result will be a new conversation in Cortex."),
                false,
            )
        }
        other => tool_text(format!("Unknown tool {other}."), true),
    }
}
