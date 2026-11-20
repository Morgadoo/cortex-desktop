//! The website chat widget: its script, CORS for the allowed sites, and its chat endpoint.

use super::*;

// The website widget

pub(super) async fn widget_script() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-cache"),
        ],
        include_str!("widget.js"),
    )
        .into_response()
}

/// The page's origin when the widget is on and the origin is allowed.
pub(super) fn widget_origin(ctx: &Ctx, headers: &HeaderMap) -> Option<String> {
    if ctx.widget.is_empty() {
        return None;
    }
    let settings = load(&ctx.config_dir).widget;
    let origin = normalize_origin(headers.get("origin").and_then(|v| v.to_str().ok())?);
    (settings.enabled && settings.origins.contains(&origin)).then_some(origin)
}

pub(super) fn with_cors(mut response: Response, origin: &str) -> Response {
    let h = response.headers_mut();
    if let Ok(v) = origin.parse() {
        h.insert("access-control-allow-origin", v);
    }
    h.insert("vary", axum::http::HeaderValue::from_static("Origin"));
    response
}

/// The page's origin as sent, while the widget is on (allowed or not: the answer then says why).
pub(super) fn any_origin(ctx: &Ctx, headers: &HeaderMap) -> Option<String> {
    if ctx.widget.is_empty() {
        return None;
    }
    headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(normalize_origin)
}

pub(super) async fn widget_preflight(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    // Any page may ask; only allowed ones are answered (the refusal is readable by the page).
    let Some(origin) = any_origin(&ctx, &headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let mut response = StatusCode::NO_CONTENT.into_response();
    let h = response.headers_mut();
    h.insert(
        "access-control-allow-methods",
        axum::http::HeaderValue::from_static("POST"),
    );
    h.insert(
        "access-control-allow-headers",
        axum::http::HeaderValue::from_static("authorization, content-type"),
    );
    h.insert(
        "access-control-max-age",
        axum::http::HeaderValue::from_static("600"),
    );
    // Chrome asks before a public page may reach this computer (Private Network Access).
    h.insert(
        "access-control-allow-private-network",
        axum::http::HeaderValue::from_static("true"),
    );
    with_cors(response, &origin)
}

#[derive(Debug, Deserialize)]
pub struct WidgetBody {
    pub messages: Vec<Value>,
}

/// The widget's conversation, made safe: only user and assistant turns (a page sets no
/// instructions), the latest few, each cut to a length, ending with the user's.
pub fn widget_messages(body: &WidgetBody) -> Result<Vec<Value>, String> {
    let mut messages: Vec<Value> = body
        .messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str)?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let text: String = content_text(m.get("content")?)
                .chars()
                .take(WIDGET_CHARS)
                .collect();
            (!text.trim().is_empty()).then(|| json!({ "role": role, "content": text }))
        })
        .collect();
    if messages.len() > WIDGET_HISTORY {
        messages.drain(..messages.len() - WIDGET_HISTORY);
    }
    match messages.last() {
        Some(m) if m["role"] == "user" => Ok(messages),
        _ => Err("Send a question.".into()),
    }
}

pub(super) async fn widget_chat(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: Result<Json<WidgetBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = std::time::Instant::now();
    let Some(origin) = widget_origin(&ctx, &headers) else {
        let asked = any_origin(&ctx, &headers).unwrap_or_default();
        record("widget", &asked, StatusCode::FORBIDDEN, started);
        let refused = error(StatusCode::FORBIDDEN, "This page may not use the Cortex widget. Add its origin in Cortex → Settings → Local API → Website widget.");
        return if asked.is_empty() {
            refused
        } else {
            with_cors(refused, &asked)
        };
    };
    let response = if !authorized(&headers, &ctx.widget) {
        error(
            StatusCode::UNAUTHORIZED,
            "The widget key is wrong; copy the snippet again from Cortex.",
        )
    } else {
        match body
            .map_err(|_| "The body must be JSON with `messages`.".to_string())
            .and_then(|Json(b)| widget_messages(&b))
        {
            Err(e) => error(StatusCode::BAD_REQUEST, &e),
            Ok(messages) => {
                let settings = load(&ctx.config_dir).widget;
                let model = if settings.model.is_empty() {
                    crate::chat::providers::load(&ctx.config_dir)
                        .default_model
                        .unwrap_or_default()
                } else {
                    settings.model
                };
                let preamble = json!({ "role": "system", "content": format!("You answer visitors of a web page ({origin}) through a chat widget. Be brief and helpful; plain text, no Markdown tables.") });
                complete(
                    &ctx,
                    ChatBody {
                        model,
                        messages: std::iter::once(preamble).chain(messages).collect(),
                        temperature: None,
                        max_tokens: None,
                        stream: true,
                    },
                )
                .await
            }
        }
    };
    record("widget", &origin, response.status(), started);
    with_cors(response, &origin)
}
