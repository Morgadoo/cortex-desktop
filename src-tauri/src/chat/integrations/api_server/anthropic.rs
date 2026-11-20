//! The Anthropic Messages API (`/v1/messages`), answered by any model set up here.

use super::*;

/// An Anthropic-style body: `system` apart, `messages` with string or block content.
#[derive(Debug, Deserialize)]
pub struct MessagesBody {
    pub model: String,
    #[serde(default)]
    pub system: Option<Value>,
    pub messages: Vec<Value>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub stream: bool,
}

/// The same request as the OpenAI shape: the system text becomes a system message.
pub fn chat_body_of(body: &MessagesBody) -> ChatBody {
    let mut messages = Vec::new();
    if let Some(system) = &body.system {
        let text = content_text(system);
        if !text.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }
    messages.extend(body.messages.iter().cloned());
    ChatBody {
        model: body.model.clone(),
        messages,
        temperature: body.temperature,
        max_tokens: body.max_tokens,
        stream: body.stream,
    }
}

pub(super) async fn messages(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: Result<Json<MessagesBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = std::time::Instant::now();
    let model = body.as_ref().map(|b| b.model.clone()).unwrap_or_default();
    let response = messages_inner(ctx, headers, body).await;
    record("messages", &model, response.status(), started);
    response
}

pub(super) fn anthropic_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "type": "error", "error": { "type": "invalid_request_error", "message": message } }))).into_response()
}

pub(super) async fn messages_inner(
    ctx: Ctx,
    headers: HeaderMap,
    body: Result<Json<MessagesBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if !authorized(&headers, &ctx.token) {
        return anthropic_error(
            StatusCode::UNAUTHORIZED,
            "Missing or wrong API key (x-api-key or bearer token).",
        );
    }
    let Ok(Json(body)) = body else {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "The body must be JSON with `model`, `messages` and `max_tokens`.",
        );
    };
    let request = match request_of(&chat_body_of(&body)) {
        Ok(r) => r,
        Err(e) => return anthropic_error(StatusCode::BAD_REQUEST, &e),
    };
    let (model, agent) = match resolve(&ctx.config_dir, &body.model).await {
        Ok(m) => m,
        Err(e) => {
            return anthropic_error(
                StatusCode::BAD_REQUEST,
                &format!("{e} Use an id from /v1/models."),
            )
        }
    };
    let request = with_agent(request, agent);
    let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    if !body.stream {
        return match model.call(request).await {
            Ok(response) => Json(json!({
                "id": id,
                "type": "message",
                "role": "assistant",
                "model": body.model,
                "content": [{ "type": "text", "text": response.text() }],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": response.usage.input_tokens.unwrap_or(0), "output_tokens": response.usage.output_tokens.unwrap_or(0) },
            }))
            .into_response(),
            Err(e) => anthropic_error(upstream_status(&e.to_string()), &crate::chat::context::short_error(&e.to_string())),
        };
    }
    let mut stream = match model.stream(request) {
        Ok(s) => s,
        Err(e) => return anthropic_error(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
    let model_name = body.model.clone();
    tauri::async_runtime::spawn(async move {
        let event = |name: &str, data: Value| Event::default().event(name).data(data.to_string());
        let _ = tx
            .send(event("message_start", json!({ "type": "message_start", "message": { "id": id, "type": "message", "role": "assistant", "model": model_name, "content": [], "stop_reason": null, "usage": { "input_tokens": 0, "output_tokens": 0 } } })))
            .await;
        let _ = tx.send(event("content_block_start", json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }))).await;
        while let Some(item) = stream.next().await {
            match item {
                Ok(Item::Event(StreamEvent::Text { text, .. })) if !text.is_empty() => {
                    if tx.send(event("content_block_delta", json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } }))).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = tx
            .send(event(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": 0 }),
            ))
            .await;
        let _ = tx.send(event("message_delta", json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn", "stop_sequence": null }, "usage": { "output_tokens": 0 } }))).await;
        let _ = tx
            .send(event("message_stop", json!({ "type": "message_stop" })))
            .await;
    });
    let events = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|e| (Ok::<_, std::convert::Infallible>(e), rx))
    });
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}
