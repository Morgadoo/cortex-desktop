//! Loopback and deep-link OAuth callbacks.
//!
//! Deep links are matched to their pending state; rmcp validates CSRF/state
//! during token exchange for both callback transports.
use crate::error::AuthError;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::broadcast;

pub const SLACK_CALLBACK_URI: &str = "proton-lumo://oauth/callback";
type CallbackResult = Result<(String, String, Option<String>), AuthError>;
fn callbacks() -> &'static broadcast::Sender<url::Url> {
    static CALLBACKS: OnceLock<broadcast::Sender<url::Url>> = OnceLock::new();
    CALLBACKS.get_or_init(|| broadcast::channel(16).0)
}

/// Checks an authorization URL before it is handed to the system opener.
///
/// The URL comes from the server's metadata, so a hostile server could otherwise launch a
/// `file:` URL or any registered protocol handler. Plain http is allowed only for loopback, for
/// local development servers.
pub fn browser_url(raw: &str) -> Result<url::Url, AuthError> {
    let url = url::Url::parse(raw)
        .map_err(|e| AuthError::Failed(format!("invalid authorization URL: {e}")))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" if url.host().is_some() => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(AuthError::Failed(
            "the server's authorization page must use https".into(),
        )),
    }
}

/// Subscribe before opening the browser; dropping the receiver cancels the waiter.
pub struct DeepLinkCallback {
    state: String,
    receiver: broadcast::Receiver<url::Url>,
}

impl DeepLinkCallback {
    pub fn register(auth_url: &str) -> Result<Self, AuthError> {
        let url = url::Url::parse(auth_url).map_err(|e| AuthError::Failed(e.to_string()))?;
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AuthError::Failed("OAuth authorization URL missing state".into()))?;
        Ok(Self {
            state,
            receiver: callbacks().subscribe(),
        })
    }

    pub async fn wait(mut self) -> CallbackResult {
        tokio::time::timeout(CALLBACK_TIMEOUT, async {
            loop {
                let url = match self.receiver.recv().await {
                    Ok(url) => url,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return Err(AuthError::Cancelled),
                };
                let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
                if params.get("state") != Some(&self.state) {
                    continue;
                }
                if let Some(error) = params.get("error") {
                    return Err(AuthError::Failed(error.clone()));
                }
                return params
                    .get("code")
                    .filter(|code| !code.is_empty())
                    .map(|code| (code.clone(), self.state.clone(), params.get("iss").cloned()))
                    .ok_or_else(|| AuthError::Failed("OAuth callback missing code".into()));
            }
        })
        .await
        .map_err(|_| AuthError::Cancelled)?
    }
}

/// Consume the MCP route even when no login is pending, keeping it out of Proton sign-in.
pub fn handle_deep_link(url: &url::Url) -> bool {
    if url.scheme() != "proton-lumo" || url.host_str() != Some("oauth") || url.path() != "/callback"
    {
        return false;
    }
    let _ = callbacks().send(url.clone());
    true
}

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const CALLBACK_PAGE: &str = include_str!("../../../assets/oauth-callback.html");
const CORTEX_MARK: &str = include_str!("../../../../webview/static/images/cortex-mark.svg");

const SUCCESS_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>"#;
const ERROR_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg>"#;

enum CallbackOutcome<'a> {
    Success,
    Error { detail: Option<&'a str> },
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_callback_page(outcome: CallbackOutcome<'_>) -> String {
    let (page_title, heading, message, status, icon, detail) = match outcome {
        CallbackOutcome::Success => (
            "Connected — Cortex",
            "You're all set",
            "Authentication was successful.",
            "success",
            SUCCESS_ICON,
            String::new(),
        ),
        CallbackOutcome::Error { detail } => (
            "Authentication failed — Cortex",
            "Something went wrong",
            "We couldn't complete the sign-in.",
            "error",
            ERROR_ICON,
            detail
                .filter(|d| !d.is_empty())
                .map(|d| format!(r#"<p class="detail">{}</p>"#, html_escape(d)))
                .unwrap_or_default(),
        ),
    };

    CALLBACK_PAGE
        .replace("{{PAGE_TITLE}}", page_title)
        .replace(
            "{{LOGO}}",
            &format!(r#"<div class="logo">{CORTEX_MARK}</div>"#),
        )
        .replace("{{STATUS}}", status)
        .replace("{{ICON}}", icon)
        .replace("{{HEADING}}", heading)
        .replace("{{MESSAGE}}", message)
        .replace("{{DETAIL}}", &detail)
}

/// Reserve the first available callback port, retaining the listener so another
/// process cannot take it between registration and the browser redirect.
pub async fn bind_oauth_callback(ports: &[u16]) -> Result<tokio::net::TcpListener, AuthError> {
    for &port in ports {
        match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => {
                return Err(AuthError::Failed(format!(
                    "failed to bind OAuth callback port {port}: {error}"
                )))
            }
        }
    }
    Err(AuthError::Failed(format!(
        "OAuth callback ports {ports:?} are all occupied"
    )))
}

/// Wait for the OAuth redirect on the reserved loopback listener and return
/// `(code, state, issuer)`. `issuer` is the RFC 9207 `iss` parameter, present
/// for authorization servers that support it (e.g. Google) and needed by rmcp
/// to validate the token exchange when the server requires it.
pub async fn wait_for_oauth_callback(
    listener: tokio::net::TcpListener,
) -> Result<(String, String, Option<String>), AuthError> {
    let timeout = tokio::time::sleep(CALLBACK_TIMEOUT);
    tokio::pin!(timeout);

    tokio::select! {
        _ = &mut timeout => Err(AuthError::Cancelled),
        accept_result = listener.accept() => {
            let (mut stream, _) = accept_result.map_err(|e| AuthError::Failed(e.to_string()))?;
            let params = read_http_request(&mut stream).await?;

            if let Some(error) = params.get("error") {
                let detail = params.get("error_description").map(String::as_str);
                let _ = write_response(
                    &mut stream,
                    render_callback_page(CallbackOutcome::Error { detail }),
                )
                .await;
                return Err(AuthError::Failed(error.clone()));
            }

            let code = params.get("code").cloned().ok_or_else(|| {
                AuthError::Failed("OAuth callback missing code".into())
            })?;
            let state = params.get("state").cloned().ok_or_else(|| {
                AuthError::Failed("OAuth callback missing state".into())
            })?;
            let issuer = params.get("iss").cloned();

            let _ = write_response(
                &mut stream,
                render_callback_page(CallbackOutcome::Success),
            )
            .await;

            Ok((code, state, issuer))
        }
    }
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<HashMap<String, String>, AuthError> {
    use tokio::io::AsyncReadExt;

    let mut buffer = vec![0u8; 4096];
    let n = stream
        .read(&mut buffer)
        .await
        .map_err(|e| AuthError::Failed(e.to_string()))?;
    let request = String::from_utf8_lossy(&buffer[..n]);

    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let query = path.split('?').nth(1).unwrap_or("");

    Ok(url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect())
}

async fn write_response(stream: &mut tokio::net::TcpStream, body: String) -> Result<(), AuthError> {
    use tokio::io::AsyncWriteExt;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| AuthError::Failed(e.to_string()))?;
    stream
        .shutdown()
        .await
        .map_err(|e| AuthError::Failed(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_are_opened_for_authorization() {
        for allowed in [
            "https://auth.example.com/authorize?state=x",
            "http://localhost:8080/authorize",
            "http://127.0.0.1:9000/authorize",
            "http://[::1]:9000/authorize",
        ] {
            assert!(browser_url(allowed).is_ok(), "{allowed}");
        }
        for refused in [
            "http://auth.example.com/authorize",
            "file:///Applications/Calculator.app",
            "smb://attacker.example/share",
            "search-ms:query=x&crumb=location:\\\\attacker.example\\share",
            "x-apple.systempreferences:com.apple.preference",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(browser_url(refused).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn deep_link_matches_state_and_preserves_issuer() {
        assert!(DeepLinkCallback::register("https://example.com/auth").is_err());
        let state = uuid::Uuid::new_v4().to_string();
        let callback =
            DeepLinkCallback::register(&format!("https://example.com/auth?state={state}")).unwrap();
        assert!(!handle_deep_link(
            &url::Url::parse(&format!("proton-lumo://login?state={state}")).unwrap()
        ));
        assert!(handle_deep_link(
            &url::Url::parse("proton-lumo://oauth/callback?state=unknown&code=bad").unwrap()
        ));
        assert!(handle_deep_link(
            &url::Url::parse(&format!(
                "{SLACK_CALLBACK_URI}?state={state}&code=valid&iss=https%3A%2F%2Fexample.com"
            ))
            .unwrap()
        ));
        assert_eq!(
            callback.wait().await.unwrap(),
            ("valid".into(), state, Some("https://example.com".into()))
        );
    }

    #[tokio::test]
    async fn deep_link_reports_provider_errors_and_missing_codes() {
        for query in ["error=access_denied", ""] {
            let state = uuid::Uuid::new_v4().to_string();
            let callback =
                DeepLinkCallback::register(&format!("https://example.com/auth?state={state}"))
                    .unwrap();
            handle_deep_link(
                &url::Url::parse(&format!("{SLACK_CALLBACK_URI}?state={state}&{query}")).unwrap(),
            );
            assert!(callback.wait().await.is_err());
        }
    }

    #[tokio::test]
    async fn callback_binding_skips_occupied_ports_and_keeps_selected_port_reserved() {
        let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let second = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let third = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ports = [
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
            third.local_addr().unwrap().port(),
        ];
        assert!(bind_oauth_callback(&ports).await.is_err());
        drop(third);
        let selected = bind_oauth_callback(&ports).await.unwrap();
        assert_eq!(selected.local_addr().unwrap().port(), ports[2]);
        assert!(
            tokio::net::TcpListener::bind(selected.local_addr().unwrap())
                .await
                .is_err()
        );
    }

    #[test]
    fn success_page_includes_branding_and_message() {
        let html = render_callback_page(CallbackOutcome::Success);
        assert!(html.contains("Connected — Cortex"));
        assert!(html.contains("You're all set"));
        assert!(html.contains("status success"));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn error_page_escapes_detail() {
        let html = render_callback_page(CallbackOutcome::Error {
            detail: Some("<script>alert(1)</script>"),
        });
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }
}
