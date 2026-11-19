use super::super::super::error::ToolError;
use base64::Engine;
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE, USER_AGENT};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(super) const USER_AGENT_VALUE: &str = concat!("Cortex-Desktop/", env!("CARGO_PKG_VERSION"));
const TOKEN_PATH: &str = "/auth/oauth2/token";
/// Refresh a client-credentials token this long before the server says it expires.
const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(60);
const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;
const MAX_ERROR_BODY_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub(super) struct CachedToken {
    pub(super) value: String,
    expires_at: Option<Instant>,
}

impl CachedToken {
    pub(super) fn is_fresh(&self) -> bool {
        self.expires_at
            .is_none_or(|at| Instant::now() + TOKEN_EXPIRY_MARGIN < at)
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

pub(super) async fn exchange_client_credentials(
    http: &reqwest::Client,
    instance_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<CachedToken, ToolError> {
    let response = http
        .post(format!("{instance_url}{TOKEN_PATH}"))
        .basic_auth(client_id, Some(client_secret))
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await
        .map_err(|e| ToolError::Unreachable(format!("Luminance token endpoint: {e}")))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ToolError::ExecutionFailed(
            if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
                "Luminance rejected the Client ID or Client Secret.".to_string()
            } else {
                format!(
                    "token request failed: {}",
                    describe_error_status(status, &body)
                )
            },
        ));
    }
    let token: TokenResponse = serde_json::from_str(&body).map_err(|_| {
        ToolError::ExecutionFailed("Luminance's token response had no access_token".into())
    })?;
    Ok(CachedToken {
        value: token.access_token,
        expires_at: token
            .expires_in
            .map(|seconds| Instant::now() + Duration::from_secs(seconds)),
    })
}

pub(super) fn header_str(response: &reqwest::Response, name: HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value: &HeaderValue| value.to_str().ok())
        .map(str::to_string)
}

pub(super) async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, ToolError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ToolError::Unreachable(format!("Luminance: {e}")))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "Luminance's response is larger than {} MB",
                MAX_RESPONSE_BYTES / (1024 * 1024)
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) fn describe_error_status(status: StatusCode, body: &str) -> String {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return "Luminance's rate limit was reached (100 requests every 10 minutes). Try again in a few minutes.".into();
    }
    let detail: String = body.trim().chars().take(MAX_ERROR_BODY_CHARS).collect();
    if detail.is_empty() {
        format!("Luminance answered {status}")
    } else {
        format!("Luminance answered {status}: {detail}")
    }
}

pub(super) fn file_result(
    content_type: Option<String>,
    disposition: Option<String>,
    bytes: &[u8],
) -> Value {
    let filename = disposition.as_deref().and_then(|value| {
        value.split(';').find_map(|part| {
            part.trim()
                .strip_prefix("filename=")
                .map(|name| name.trim_matches('"').to_string())
        })
    });
    let textual = content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("text/") || ct.contains("json") || ct.contains("xml"));
    let (encoding, content) = match std::str::from_utf8(bytes) {
        Ok(text) if textual => ("utf8", text.to_string()),
        _ => (
            "base64",
            base64::engine::general_purpose::STANDARD.encode(bytes),
        ),
    };
    json!({
        "filename": filename,
        "content_type": content_type,
        "size_bytes": bytes.len(),
        "encoding": encoding,
        "content": content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_textual_download_is_returned_as_text_and_anything_else_as_base64() {
        let text = file_result(
            Some("text/plain".into()),
            Some("attachment; filename=\"a.txt\"".into()),
            b"hello",
        );
        assert_eq!(text["encoding"], "utf8");
        assert_eq!(text["content"], "hello");
        assert_eq!(text["filename"], "a.txt");

        let binary = file_result(Some("application/pdf".into()), None, b"%PDF");
        assert_eq!(binary["encoding"], "base64");
        assert_eq!(binary["content"], "JVBERg==");
    }
}
