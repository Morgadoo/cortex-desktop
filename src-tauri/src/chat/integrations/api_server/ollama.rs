//! The Ollama API (`/api/chat`, `/api/generate`, `/api/tags`), for apps that speak only Ollama.

use super::*;

pub(super) fn ollama_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub(super) async fn ollama_version(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &ctx.token) {
        return ollama_error(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    }
    Json(json!({ "version": env!("CARGO_PKG_VERSION") })).into_response()
}

pub(super) async fn ollama_tags(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &ctx.token) {
        return ollama_error(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    }
    let now = chrono::Utc::now().to_rfc3339();
    let file = crate::chat::providers::load(&ctx.config_dir);
    let entry = |name: String, family: &str| json!({ "name": name, "model": name, "modified_at": now, "size": 0, "digest": "", "details": { "format": "remote", "family": family, "families": [family], "parameter_size": "", "quantization_level": "" } });
    let models: Vec<Value> = crate::chat::providers::models(&file)
        .await
        .into_iter()
        .map(|m| entry(m.reference, &m.vendor))
        .chain(
            crate::chat::agents::load(&ctx.config_dir)
                .into_iter()
                .map(|a| entry(format!("{AGENT_PREFIX}{}", a.id), "cortex-agent")),
        )
        .collect();
    Json(json!({ "models": models })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct OllamaBody {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    /// Ollama streams unless told not to.
    #[serde(default = "yes")]
    pub stream: bool,
    #[serde(default)]
    pub options: Option<Value>,
}

pub(super) fn yes() -> bool {
    true
}

/// The Chat Completions shape of an Ollama `/api/chat` or `/api/generate` body.
pub(in crate::chat::integrations) fn chat_body_of_ollama(body: &OllamaBody) -> ChatBody {
    let mut messages = Vec::new();
    if let Some(s) = body.system.as_ref().filter(|s| !s.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": s }));
    }
    messages.extend(body.messages.iter().map(|m| json!({ "role": m.get("role").cloned().unwrap_or(json!("user")), "content": m.get("content").cloned().unwrap_or(json!("")) })));
    if let Some(p) = &body.prompt {
        messages.push(json!({ "role": "user", "content": p }));
    }
    let option = |k: &str| body.options.as_ref().and_then(|o| o.get(k)).cloned();
    ChatBody {
        model: body.model.clone(),
        messages,
        temperature: option("temperature").and_then(|v| v.as_f64()),
        max_tokens: option("num_predict").and_then(|v| v.as_u64()),
        stream: body.stream,
    }
}

// Ollama reads the body as JSON whatever its content type (its own `curl -d` examples send none).
pub(super) async fn ollama_chat(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    ollama(ctx, headers, serde_json::from_slice(&body), true).await
}

pub(super) async fn ollama_generate(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    ollama(ctx, headers, serde_json::from_slice(&body), false).await
}

/// `/api/chat` answers with `message`, `/api/generate` with `response`; both stream NDJSON lines
/// and end with `done: true`.
pub(super) async fn ollama(
    ctx: Ctx,
    headers: HeaderMap,
    body: Result<OllamaBody, serde_json::Error>,
    chat: bool,
) -> Response {
    let started = std::time::Instant::now();
    let log = ctx.log.clone();
    let endpoint = if chat { "api/chat" } else { "api/generate" };
    let model_name = body.as_ref().map(|b| b.model.clone()).unwrap_or_default();
    let response = async {
        if !authorized(&headers, &ctx.token) {
            return ollama_error(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
        }
        let Ok(body) = body else {
            return ollama_error(StatusCode::BAD_REQUEST, "the body must be JSON with `model` and `messages` (or `prompt`)");
        };
        let request = match request_of(&chat_body_of_ollama(&body)) {
            Ok(r) => r,
            Err(e) => return ollama_error(StatusCode::BAD_REQUEST, &e),
        };
        let (model, agent) = match resolve(&ctx.config_dir, &body.model).await {
            Ok(m) => m,
            Err(e) => return ollama_error(StatusCode::NOT_FOUND, &format!("{e} Use a name from /api/tags.")),
        };
        let request = with_agent(request, agent);
        let name = body.model.clone();
        let line = move |text: &str, done: bool| {
            let mut v = json!({ "model": name, "created_at": chrono::Utc::now().to_rfc3339(), "done": done });
            if chat {
                v["message"] = json!({ "role": "assistant", "content": text });
            } else {
                v["response"] = json!(text);
            }
            if done {
                v["done_reason"] = json!("stop");
            }
            v
        };
        if !body.stream {
            return match model.call(request).await {
                Ok(r) => Json(line(&r.text(), true)).into_response(),
                Err(e) => ollama_error(upstream_status(&e.to_string()), &crate::chat::context::short_error(&e.to_string())),
            };
        }
        let mut stream = match model.stream(request) {
            Ok(s) => s,
            Err(e) => return ollama_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::convert::Infallible>>(64);
        tauri::async_runtime::spawn(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(Item::Event(StreamEvent::Text { text, .. })) if !text.is_empty() => {
                        if tx.send(Ok(format!("{}\n", line(&text, false)))).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(Ok(format!("{}\n", line("", true)))).await;
        });
        let lines = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|l| (l, rx)) });
        Response::builder().header("content-type", "application/x-ndjson").body(axum::body::Body::from_stream(lines)).unwrap_or_else(|_| ollama_error(StatusCode::INTERNAL_SERVER_ERROR, "could not stream"))
    }
    .await;
    record(&log, endpoint, &model_name, response.status(), started);
    response
}
