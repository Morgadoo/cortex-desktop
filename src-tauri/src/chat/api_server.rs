//! A local, OpenAI-compatible API, as Osaurus offers: scripts, editors and other apps on this
//! computer can use the models set up here (any provider, keys stay in the keychain) through the
//! usual `/v1/chat/completions` and `/v1/models` endpoints.
//!
//! Off by default. When on, it listens on 127.0.0.1 only and every request needs the bearer
//! token shown in Settings (kept in the keychain as `api-server`). It sends no CORS headers, so
//! web pages cannot call it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use rig_core::completion::message::Message;
use rig_core::completion::CompletionRequest;
use rig_core::streaming::{Item, StreamEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::store::write_json;

pub const SECRET: &str = "api-server";
pub const DEFAULT_PORT: u16 = 8484;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ApiServerSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for ApiServerSettings {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT }
    }
}

fn settings_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("api-server.json")
}

pub fn load(config_dir: &Path) -> ApiServerSettings {
    std::fs::read_to_string(settings_path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &ApiServerSettings) -> Result<(), String> {
    if settings.port < 1024 {
        return Err("Choose a port from 1024 up.".into());
    }
    write_json(&settings_path(config_dir), settings)
}

fn secrets() -> Result<crate::tools::auth::Secrets, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    Ok(crate::tools::auth::keychain_secrets(account))
}

/// The token requests must carry, created on first use.
pub async fn token(regenerate: bool) -> Result<String, String> {
    let secrets = secrets()?;
    if !regenerate {
        if let Some(existing) = secrets.load(SECRET).await?.filter(|t| !t.is_empty()) {
            return Ok(existing);
        }
    }
    let fresh = format!("lumo-{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    secrets.store(SECRET, &fresh).await?;
    Ok(fresh)
}

/// The running server, if any: stopping it is sending on this channel.
#[derive(Default)]
pub struct ApiServer {
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    status: Mutex<Status>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub address: Option<String>,
    pub error: Option<String>,
}

impl ApiServer {
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, status: Status) {
        if let Ok(mut s) = self.status.lock() {
            *s = status;
        }
    }

    fn stop(&self) {
        if let Some(stop) = self.stop.lock().ok().and_then(|mut s| s.take()) {
            let _ = stop.send(());
        }
    }
}

/// Starts, restarts or stops the server to match the saved settings.
pub async fn apply(app: &AppHandle) -> Status {
    let Some(server) = app.try_state::<ApiServer>() else { return Status::default() };
    server.stop();
    let Ok(config_dir) = crate::settings::get_config_dir(app) else { return Status::default() };
    let settings = load(&config_dir);
    if !settings.enabled {
        server.set(Status::default());
        return server.status();
    }
    let token = match token(false).await {
        Ok(t) => t,
        Err(e) => {
            server.set(Status { running: false, address: None, error: Some(e) });
            return server.status();
        }
    };
    let address = format!("127.0.0.1:{}", settings.port);
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(l) => l,
        Err(e) => {
            server.set(Status { running: false, address: None, error: Some(format!("Port {} is not free ({e}).", settings.port)) });
            return server.status();
        }
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    if let Ok(mut s) = server.stop.lock() {
        *s = Some(stop);
    }
    let router = router(Ctx { config_dir, token: Arc::new(token) });
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            })
            .await;
    });
    server.set(Status { running: true, address: Some(format!("http://{address}/v1")), error: None });
    log::info!("local API listening on {address}");
    server.status()
}

#[derive(Clone)]
struct Ctx {
    config_dir: PathBuf,
    token: Arc<String>,
}

fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat))
        .with_state(ctx)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": { "message": message, "type": "invalid_request_error" } }))).into_response()
}

/// Compares in time that does not depend on where the strings differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| same(given.trim(), token))
}

async fn models(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &ctx.token) {
        return error(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.");
    }
    let file = super::providers::load(&ctx.config_dir);
    let data: Vec<Value> = super::providers::models(&file)
        .await
        .into_iter()
        .map(|m| json!({ "id": m.reference, "object": "model", "owned_by": m.vendor, "name": m.name }))
        .collect();
    Json(json!({ "object": "list", "data": data })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ChatBody {
    pub model: String,
    pub messages: Vec<Value>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default, alias = "max_completion_tokens")]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub stream: bool,
}

/// The text of a message's `content`: a string, or an array of `{type: "text", text}` parts.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The request for an OpenAI-style body: system messages become the preamble, the last user
/// message the prompt, the rest the history.
pub fn request_of(body: &ChatBody) -> Result<CompletionRequest, String> {
    let mut system: Vec<String> = Vec::new();
    let mut turns: Vec<Message> = Vec::new();
    for m in &body.messages {
        let text = content_text(m.get("content").unwrap_or(&Value::Null));
        match m.get("role").and_then(Value::as_str).unwrap_or_default() {
            "system" | "developer" => system.push(text),
            "user" => turns.push(Message::user(text)),
            "assistant" => turns.push(Message::assistant(text)),
            other => return Err(format!("Messages with role `{other}` are not supported.")),
        }
    }
    let Some(prompt) = turns.pop() else {
        return Err("Send at least one user message.".into());
    };
    if !matches!(prompt, Message::User { .. }) {
        return Err("The last message must be from the user.".into());
    }
    let mut request = CompletionRequest::new(prompt).messages(turns);
    if !system.is_empty() {
        request = request.preamble(system.join("\n\n"));
    }
    if let Some(t) = body.temperature {
        request = request.temperature(t.clamp(0.0, 2.0));
    }
    if let Some(n) = body.max_tokens {
        request = request.max_tokens(n);
    }
    Ok(request)
}

async fn chat(State(ctx): State<Ctx>, headers: HeaderMap, body: Result<Json<ChatBody>, axum::extract::rejection::JsonRejection>) -> Response {
    if !authorized(&headers, &ctx.token) {
        return error(StatusCode::UNAUTHORIZED, "Missing or wrong bearer token.");
    }
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "The body must be JSON with `model` and `messages`.");
    };
    let request = match request_of(&body) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e),
    };
    let model = match super::engine::connect_model(&ctx.config_dir, &body.model).await {
        Ok(m) => m,
        Err(e) => return error(StatusCode::BAD_REQUEST, &format!("{e} Use an id from /v1/models.")),
    };
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let created = chrono::Utc::now().timestamp();
    if !body.stream {
        return match model.call(request).await {
            Ok(response) => Json(json!({
                "id": id,
                "object": "chat.completion",
                "created": created,
                "model": body.model,
                "choices": [{ "index": 0, "message": { "role": "assistant", "content": response.text() }, "finish_reason": "stop" }],
                "usage": {
                    "prompt_tokens": response.usage.input_tokens.unwrap_or(0),
                    "completion_tokens": response.usage.output_tokens.unwrap_or(0),
                    "total_tokens": response.usage.input_tokens.unwrap_or(0) + response.usage.output_tokens.unwrap_or(0),
                },
            }))
            .into_response(),
            Err(e) => error(StatusCode::BAD_GATEWAY, &super::context::short_error(&e.to_string())),
        };
    }
    let mut stream = match model.stream(request) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
    let model_name = body.model.clone();
    tauri::async_runtime::spawn(async move {
        let chunk = |delta: Value, finish: Option<&str>| {
            Event::default().data(
                json!({ "id": id, "object": "chat.completion.chunk", "created": created, "model": model_name, "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
                    .to_string(),
            )
        };
        let _ = tx.send(chunk(json!({ "role": "assistant" }), None)).await;
        while let Some(item) = stream.next().await {
            match item {
                Ok(Item::Event(StreamEvent::Text { text, .. })) if !text.is_empty() => {
                    if tx.send(chunk(json!({ "content": text }), None)).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = tx.send(chunk(json!({}), Some("stop"))).await;
        let _ = tx.send(Event::default().data("[DONE]")).await;
    });
    let events = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (Ok::<_, std::convert::Infallible>(e), rx)) });
    Sse::new(events).keep_alive(KeepAlive::default()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_bodies_become_requests() {
        let body: ChatBody = serde_json::from_value(json!({
            "model": "xai/grok-4",
            "messages": [
                { "role": "system", "content": "Be brief." },
                { "role": "user", "content": "Hi" },
                { "role": "assistant", "content": "Hello!" },
                { "role": "user", "content": [{ "type": "text", "text": "Two plus two?" }] }
            ],
            "temperature": 5.0,
            "max_completion_tokens": 50
        }))
        .unwrap();
        let request = request_of(&body).unwrap();
        assert_eq!(request.system_instructions(), Some("Be brief."));
        assert_eq!(request.temperature, Some(2.0), "temperature is clamped");
        assert_eq!(request.max_tokens, Some(50));
        let bad: ChatBody = serde_json::from_value(json!({ "model": "x/y", "messages": [{ "role": "assistant", "content": "hi" }] })).unwrap();
        assert!(request_of(&bad).is_err());
        let tool: ChatBody = serde_json::from_value(json!({ "model": "x/y", "messages": [{ "role": "tool", "content": "{}" }] })).unwrap();
        assert!(request_of(&tool).is_err());
    }

    #[test]
    fn tokens_are_checked_exactly() {
        let mut headers = HeaderMap::new();
        assert!(!authorized(&headers, "lumo-abc"));
        headers.insert("authorization", "Bearer lumo-abc".parse().unwrap());
        assert!(authorized(&headers, "lumo-abc"));
        assert!(!authorized(&headers, "lumo-abd"));
        assert!(!authorized(&headers, "lumo-ab"));
        assert!(same("", "") && !same("a", "b"));
    }

    #[test]
    fn settings_default_off() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), ApiServerSettings { enabled: false, port: DEFAULT_PORT });
        assert!(save(dir.path(), &ApiServerSettings { enabled: true, port: 80 }).is_err());
    }
}
