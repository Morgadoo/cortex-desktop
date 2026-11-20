//! The OpenAI Responses API (`/v1/responses`).

use super::*;

#[derive(Debug, Deserialize)]
pub struct ResponsesBody {
    pub model: String,
    /// A string, or a list of messages (`{role, content}` with text or `input_text` parts).
    pub input: Value,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub stream: bool,
}

/// The same request as the Chat Completions shape.
pub(in crate::chat::integrations) fn chat_body_of_responses(body: &ResponsesBody) -> ChatBody {
    let mut messages = Vec::new();
    if let Some(i) = body.instructions.as_ref().filter(|i| !i.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": i }));
    }
    match &body.input {
        Value::String(s) => messages.push(json!({ "role": "user", "content": s })),
        Value::Array(items) => {
            for item in items {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                messages.push(json!({ "role": role, "content": content_text(item.get("content").unwrap_or(&Value::Null)) }));
            }
        }
        _ => {}
    }
    ChatBody {
        model: body.model.clone(),
        messages,
        temperature: body.temperature,
        max_tokens: body.max_output_tokens,
        stream: body.stream,
    }
}

pub(super) async fn responses(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: Result<Json<ResponsesBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = std::time::Instant::now();
    let model_name = body.as_ref().map(|b| b.model.clone()).unwrap_or_default();
    let response = async {
        if !authorized(&headers, &ctx.token) {
            return error(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.");
        }
        let Ok(Json(body)) = body else {
            return error(StatusCode::BAD_REQUEST, "The body must be JSON with `model` and `input`.");
        };
        let request = match request_of(&chat_body_of_responses(&body)) {
            Ok(r) => r,
            Err(e) => return error(StatusCode::BAD_REQUEST, &e),
        };
        let (model, agent) = match resolve(&ctx.config_dir, &body.model).await {
            Ok(m) => m,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("{e} Use an id from /v1/models.")),
        };
        let request = with_agent(request, agent);
        let id = format!("resp_{}", uuid::Uuid::new_v4().simple());
        let created = chrono::Utc::now().timestamp();
        let (response_id, model_id) = (id.clone(), body.model.clone());
        let finished = move |text: &str, input: u64, output: u64| {
            json!({
                "id": response_id, "object": "response", "created_at": created, "status": "completed", "model": model_id,
                "output": [{ "type": "message", "id": format!("msg_{}", &response_id[5..]), "status": "completed", "role": "assistant", "content": [{ "type": "output_text", "text": text, "annotations": [] }] }],
                "output_text": text,
                "usage": { "input_tokens": input, "output_tokens": output, "total_tokens": input + output },
            })
        };
        if !body.stream {
            return match model.call(request).await {
                Ok(r) => Json(finished(&r.text(), r.usage.input_tokens.unwrap_or(0), r.usage.output_tokens.unwrap_or(0))).into_response(),
                Err(e) => error(upstream_status(&e.to_string()), &crate::chat::context::short_error(&e.to_string())),
            };
        }
        let mut stream = match model.stream(request) {
            Ok(s) => s,
            Err(e) => return error(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
        let shell = json!({ "id": id, "object": "response", "created_at": created, "status": "in_progress", "model": body.model, "output": [] });
        tauri::async_runtime::spawn(async move {
            let event = |kind: &str, data: Value| Event::default().event(kind).data(data.to_string());
            let _ = tx.send(event("response.created", json!({ "type": "response.created", "response": shell }))).await;
            let mut text = String::new();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(Item::Event(StreamEvent::Text { text: delta, .. })) if !delta.is_empty() => {
                        text.push_str(&delta);
                        if tx.send(event("response.output_text.delta", json!({ "type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": delta }))).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(event("response.output_text.done", json!({ "type": "response.output_text.done", "output_index": 0, "content_index": 0, "text": text }))).await;
            let _ = tx.send(event("response.completed", json!({ "type": "response.completed", "response": finished(&text, 0, 0) }))).await;
        });
        let events = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (Ok::<_, std::convert::Infallible>(e), rx)) });
        Sse::new(events).keep_alive(KeepAlive::default()).into_response()
    }
    .await;
    record("responses", &model_name, response.status(), started);
    response
}
