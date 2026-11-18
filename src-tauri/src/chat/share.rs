//! Share links (Vibe's public share link), without a cloud: a conversation or an app shared as a
//! self-contained page, served by Lumo on this computer's network at
//! `http://<this computer>:8485/s/<random token>`. Anyone on the same network who has the link can
//! open it while Lumo runs; stopping a share takes the page down.
//!
//! The server only serves shared pages (nothing else of the app is reachable), each under a
//! 128-bit random token, with a Content-Security-Policy that lets the page load nothing and run no
//! script. It starts with the first share and, after a restart, when shares are left. Shares are
//! kept in `chat/shares/` (the page and a list of what is shared).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use log::{info, warn};
use serde::{Deserialize, Serialize};

pub const PORT: u16 = 8485;
/// A conversation page: it loads nothing and runs no script.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data: https:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// A mini app: its own inline script and styles run, but it reaches nothing over the network.
const APP_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub token: String,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub views: u64,
    /// An app (its script runs) rather than a conversation page.
    #[serde(default)]
    pub app: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLink {
    #[serde(flatten)]
    pub share: Share,
    /// The address others on the network open.
    pub url: String,
    /// The same page on this computer.
    pub local_url: String,
}

fn dir(config_dir: &Path) -> PathBuf {
    config_dir.join("chat").join("shares")
}

pub fn list(config_dir: &Path) -> Vec<Share> {
    crate::chat::at_rest::read_to_string(dir(config_dir).join("shares.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_list(config_dir: &Path, shares: &[Share]) -> Result<(), String> {
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(dir(config_dir).join("shares.json"), serde_json::to_string_pretty(shares).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// A token is 32 lowercase hex characters; anything else is never a file name here.
pub fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// This computer's address on its network: the one the system would use to reach the internet
/// (a UDP "connect" sends nothing). Loopback when there is no network.
pub fn lan_address() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_unspecified())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn link(share: Share, port: u16) -> SharedLink {
    let url = format!("http://{}:{port}/s/{}", lan_address(), share.token);
    let local_url = format!("http://127.0.0.1:{port}/s/{}", share.token);
    SharedLink { share, url, local_url }
}

/// The port the share server listens on, once started.
static RUNNING: Mutex<Option<u16>> = Mutex::new(None);

/// Starts the server unless it runs; returns its port.
pub async fn ensure_server(config_dir: &Path) -> Result<u16, String> {
    if let Some(port) = *RUNNING.lock().unwrap() {
        return Ok(port);
    }
    let listener = match tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], PORT))).await {
        Ok(l) => l,
        // Another program has the usual port: any free one does.
        Err(_) => tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await.map_err(|e| format!("Sharing could not start: {e}"))?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let app = Router::new().route("/s/{token}", get(page)).with_state(config_dir.to_path_buf());
    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!("share server stopped: {e}");
        }
        *RUNNING.lock().unwrap() = None;
    });
    *RUNNING.lock().unwrap() = Some(port);
    info!("sharing on port {port}");
    Ok(port)
}

async fn page(State(config_dir): State<PathBuf>, UrlPath(token): UrlPath<String>) -> Response {
    let not_found = || (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "This shared page is not available (it was stopped, or the link is wrong).").into_response();
    if !valid_token(&token) {
        return not_found();
    }
    let mut shares = list(&config_dir);
    let Some(share) = shares.iter_mut().find(|s| s.token == token) else { return not_found() };
    let Ok(html) = crate::chat::at_rest::read_to_string(dir(&config_dir).join(format!("{token}.html"))) else { return not_found() };
    share.views += 1;
    let csp = if share.app { APP_CSP } else { CSP };
    let _ = write_list(&config_dir, &shares);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, csp),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response()
}

/// Shares `html` under a new link.
pub async fn share(config_dir: &Path, title: &str, html: &str, app: bool) -> Result<SharedLink, String> {
    if html.len() > 64 * 1024 * 1024 {
        return Err("That is too large to share as one page.".into());
    }
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(dir(config_dir).join(format!("{token}.html")), html).map_err(|e| e.to_string())?;
    let share = Share { token, title: title.trim().chars().take(200).collect(), created_at: chrono::Utc::now(), views: 0, app };
    let mut shares = list(config_dir);
    shares.insert(0, share.clone());
    write_list(config_dir, &shares)?;
    let port = ensure_server(config_dir).await?;
    Ok(link(share, port))
}

pub async fn links(config_dir: &Path) -> Vec<SharedLink> {
    let shares = list(config_dir);
    if shares.is_empty() {
        return Vec::new();
    }
    let port = ensure_server(config_dir).await.unwrap_or(PORT);
    shares.into_iter().map(|s| link(s, port)).collect()
}

pub fn stop(config_dir: &Path, token: &str) -> Result<(), String> {
    if !valid_token(token) {
        return Err("No such share.".into());
    }
    let mut shares = list(config_dir);
    shares.retain(|s| s.token != token);
    write_list(config_dir, &shares)?;
    let _ = std::fs::remove_file(dir(config_dir).join(format!("{token}.html")));
    Ok(())
}

/// At start: serve what is still shared.
pub fn start(app: &tauri::AppHandle) {
    let Ok(config_dir) = crate::settings::get_config_dir(app) else { return };
    if list(&config_dir).is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = ensure_server(&config_dir).await {
            warn!("{e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_strict() {
        assert!(valid_token("0123456789abcdef0123456789abcdef"));
        for bad in ["", "../shares", "0123456789ABCDEF0123456789abcdef", "0123456789abcdef0123456789abcde", "0123456789abcdef0123456789abcdeg"] {
            assert!(!valid_token(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_shared_page_is_served_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let shared = share(dir.path(), "Trip plan", "<h1>Porto</h1>", false).await.unwrap();
        assert!(shared.url.contains(&shared.share.token) && shared.local_url.starts_with("http://127.0.0.1:"));
        let body = reqwest::get(&shared.local_url).await.unwrap();
        assert_eq!(body.status(), 200);
        assert!(body.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
        assert_eq!(body.text().await.unwrap(), "<h1>Porto</h1>");
        assert_eq!(list(dir.path())[0].views, 1);
        let app = share(dir.path(), "Counter", "<script>1</script>", true).await.unwrap();
        let csp = reqwest::get(&app.local_url).await.unwrap().headers()["content-security-policy"].to_str().unwrap().to_string();
        assert!(csp.contains("script-src 'unsafe-inline'") && csp.contains("connect-src 'none'"));
        let base = shared.local_url.rsplit_once("/s/").unwrap().0;
        assert_eq!(reqwest::get(format!("{base}/s/0123456789abcdef0123456789abcdef")).await.unwrap().status(), 404);
        assert_eq!(reqwest::get(format!("{base}/")).await.unwrap().status(), 404, "nothing else is served");
        stop(dir.path(), &shared.share.token).unwrap();
        assert_eq!(reqwest::get(&shared.local_url).await.unwrap().status(), 404);
    }
}
