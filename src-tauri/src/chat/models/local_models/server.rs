//! The endpoint the engine talks to: OpenAI's chat completions, on 127.0.0.1 only.

use super::*;

// The endpoint the engine talks to: OpenAI's chat completions, on 127.0.0.1 only.

pub(super) static PORT: tokio::sync::OnceCell<u16> = tokio::sync::OnceCell::const_new();

/// The port of the built-in endpoint, started on first use.
pub(in crate::chat::models) async fn endpoint() -> Result<u16, String> {
    PORT.get_or_try_init(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let app = axum::Router::new()
            .route("/v1/chat/completions", axum::routing::post(completions))
            .route("/v1/models", axum::routing::get(list_models));
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                log::warn!("the built-in model endpoint stopped: {e}");
            }
        });
        Ok::<u16, String>(port)
    })
    .await
    .copied()
}

pub(super) async fn list_models() -> axum::Json<Value> {
    let ids: Vec<Value> = root()
        .map(|r| models(&r))
        .unwrap_or_default()
        .into_iter()
        .map(|m| json!({ "id": m.id, "object": "model", "owned_by": "cortex" }))
        .collect();
    axum::Json(json!({ "object": "list", "data": ids }))
}

pub(super) fn error_response(status: u16, message: &str) -> axum::response::Response {
    let body =
        json!({ "error": { "message": message, "type": "invalid_request_error" } }).to_string();
    axum::response::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap_or_default()
}

/// Requests waiting for or using the processor. One model runs at a time; the rest wait in
/// arrival order (the model's lock is fair), and past `QUEUE_LIMIT` a request is told to retry.
pub(super) static QUEUED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub const QUEUE_LIMIT: usize = 8;

/// Holds a place in the queue for as long as it lives.
pub struct Place;

impl Place {
    /// A place, or `None` when the queue is full.
    pub fn take() -> Option<Place> {
        use std::sync::atomic::Ordering;
        let mut now = QUEUED.load(Ordering::SeqCst);
        loop {
            if now >= QUEUE_LIMIT {
                return None;
            }
            match QUEUED.compare_exchange(now, now + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Place),
                Err(actual) => now = actual,
            }
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        QUEUED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) async fn completions(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    let Ok(root) = root() else {
        return error_response(500, "The model folder is not set up.");
    };
    let Some(place) = Place::take() else {
        let body = json!({ "error": { "message": format!("The built-in runtime is busy ({QUEUE_LIMIT} requests waiting); try again shortly.", ), "type": "rate_limit_error" } }).to_string();
        return axum::response::Response::builder()
            .status(429)
            .header("content-type", "application/json")
            .header("retry-after", "5")
            .body(axum::body::Body::from(body))
            .unwrap_or_default();
    };
    let id = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim_start_matches("local/")
        .to_string();
    if !installed(&root, &id) {
        return error_response(404, &format!("The model `{id}` is not downloaded."));
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let params = Params {
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        max_tokens: body
            .get("max_completion_tokens")
            .or_else(|| body.get("max_tokens"))
            .and_then(Value::as_u64)
            .map(|n| n as usize),
    };
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    log::info!(
        "built-in model {id}: {} messages, {} characters, {} tools offered, stream {stream}",
        messages.len(),
        messages
            .iter()
            .map(|m| content_text(m.get("content").unwrap_or(&Value::Null)).len())
            .sum::<usize>(),
        body.get("tools")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    let created = chrono::Utc::now().timestamp();
    let rid = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let model_id = id.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _place = place;
        let tx_text = tx.clone();
        let result = generate(&root, &model_id, &messages, &params, |piece| {
            tx_text.send(Ok(piece.to_string())).is_ok()
        });
        result.inspect_err(|e| {
            let _ = tx.send(Err(e.clone()));
        })
    });
    if !stream {
        let mut rx = rx;
        let mut text = String::new();
        while let Some(piece) = rx.recv().await {
            match piece {
                Ok(p) => text.push_str(&p),
                Err(e) => return error_response(500, &e),
            }
        }
        let usage = match task.await {
            Ok(Ok(u)) => u,
            Ok(Err(e)) => return error_response(500, &e),
            Err(e) => return error_response(500, &e.to_string()),
        };
        let body = json!({
            "id": rid, "object": "chat.completion", "created": created, "model": id,
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": usage.prompt, "completion_tokens": usage.completion, "total_tokens": usage.prompt + usage.completion }
        });
        return axum::response::IntoResponse::into_response(axum::Json(body));
    }
    // Server-sent events, as OpenAI streams them.
    let chunk = move |delta: Value, finish: Option<&str>, usage: Option<Value>| {
        let mut c = json!({ "id": rid, "object": "chat.completion.chunk", "created": created, "model": id, "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] });
        if let Some(u) = usage {
            c["usage"] = u;
            c["choices"] = json!([]);
        }
        format!("data: {c}\n\n")
    };
    let state = (rx, Some(task), false, chunk);
    let events = futures::stream::unfold(state, |(mut rx, task, started, chunk)| async move {
        if !started {
            let first = chunk(json!({ "role": "assistant", "content": "" }), None, None);
            return Some((
                Ok::<_, std::io::Error>(axum::body::Bytes::from(first)),
                (rx, task, true, chunk),
            ));
        }
        match rx.recv().await {
            Some(Ok(piece)) => Some((
                Ok(axum::body::Bytes::from(chunk(
                    json!({ "content": piece }),
                    None,
                    None,
                ))),
                (rx, task, true, chunk),
            )),
            Some(Err(e)) => {
                let err = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({ "error": { "message": e } })
                );
                Some((Ok(axum::body::Bytes::from(err)), (rx, None, true, chunk)))
            }
            None => {
                let task = task?;
                let usage = match task.await {
                    Ok(Ok(u)) => {
                        json!({ "prompt_tokens": u.prompt, "completion_tokens": u.completion, "total_tokens": u.prompt + u.completion })
                    }
                    _ => json!({ "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }),
                };
                let end = format!(
                    "{}{}data: [DONE]\n\n",
                    chunk(json!({}), Some("stop"), None),
                    chunk(json!({}), None, Some(usage))
                );
                Some((Ok(axum::body::Bytes::from(end)), (rx, None, true, chunk)))
            }
        }
    });
    axum::response::Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(events))
        .unwrap_or_default()
}

/// The system prompt of a built-in model: who it is, today's date, and the user's own
/// instructions; short, because a small model on a processor reads every token.
pub fn brief(
    instructions: &crate::chat::store::Instructions,
    project: Option<&str>,
    agent: Option<(&str, &str)>,
) -> String {
    let mut out = match agent {
        Some((name, text)) => format!("You are {name}, running on the user's own computer. {}", text.trim()),
        None => "You are Cortex, a helpful assistant running on the user's own computer. Answer clearly and briefly; use Markdown when it helps.".to_string(),
    };
    out.push_str(&format!(
        "\nToday is {}.",
        chrono::Local::now().format("%A, %-d %B %Y")
    ));
    if !instructions.name.trim().is_empty() {
        out.push_str(&format!(
            "\nThe user's name is {}.",
            instructions.name.trim()
        ));
    }
    if !instructions.tone.trim().is_empty() {
        out.push_str(&format!("\nTone: {}.", instructions.tone.trim()));
    }
    for extra in [
        instructions.custom.trim(),
        project.unwrap_or_default().trim(),
    ] {
        if !extra.is_empty() {
            out.push_str("\n\n");
            out.push_str(&extra.chars().take(2_000).collect::<String>());
        }
    }
    out
}

/// The model a `local/<id>` reference runs on, through the built-in endpoint.
pub(in crate::chat::models) async fn connect(
    reference: &str,
) -> Result<rig_core::DynModel<rig_core::operation::Completion>, String> {
    let id = reference
        .strip_prefix(VENDOR)
        .and_then(|r| r.strip_prefix('/'))
        .ok_or("Not a built-in model.")?;
    let root = root()?;
    if !installed(&root, id) {
        return Err(format!(
            "Download {} first (Settings → Providers → Built-in models).",
            entry(id).map_or(id, |e| e.name)
        ));
    }
    let port = endpoint().await?;
    let settings = crate::chat::providers::ProviderSettings {
        enabled: true,
        base_url: Some(format!("http://127.0.0.1:{port}/v1")),
        ..Default::default()
    };
    crate::chat::providers::connect(&format!("llamacpp/{id}"), &settings, None)
}
