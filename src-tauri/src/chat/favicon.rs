//! Site icons for sources and citations, as Vibe shows them next to each source.
//!
//! The page may only load images from itself (its CSP), so the app fetches a site's icon once,
//! keeps it in `chat/favicons/`, and hands it to the page as a `data:` URL. The site's own
//! `/favicon.ico` comes first, then the icon its home page declares. A site without one is
//! remembered for a week so it is not asked again on every render.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine as _;

use super::store::chat_dir;

/// Largest icon kept.
const MAX_ICON_BYTES: usize = 200 * 1024;
/// Of a home page, the part read to find its icon link.
const MAX_PAGE_BYTES: usize = 256 * 1024;
/// How long "this site has no icon" is remembered.
const MISS_TTL: Duration = Duration::from_secs(7 * 86_400);

/// A host name as it may appear in a URL: letters, digits, dots and hyphens.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host.starts_with('.')
        && !host.contains("..")
}

fn cache_path(config_dir: &Path, host: &str) -> PathBuf {
    chat_dir(config_dir)
        .join("favicons")
        .join(format!("{host}.txt"))
}

/// What kind of image the bytes are, by their first bytes or the server's word for it.
fn image_type(bytes: &[u8], content_type: &str) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) {
        return Some("image/x-icon");
    }
    if bytes.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if head.contains("<svg") || content_type.contains("svg") {
        return Some("image/svg+xml");
    }
    None
}

/// The icon a home page declares: `<link rel="icon" href="…">` and its variants.
pub fn icon_link(html: &str) -> Option<String> {
    let tag = regex::Regex::new(r#"(?is)<link\b[^>]*>"#).ok()?;
    let rel = regex::Regex::new(r#"(?is)\brel\s*=\s*["']([^"']*)["']"#).ok()?;
    let href = regex::Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#).ok()?;
    let mut best: Option<(u8, String)> = None;
    for m in tag.find_iter(html) {
        let t = m.as_str();
        let Some(r) = rel.captures(t).map(|c| c[1].to_ascii_lowercase()) else {
            continue;
        };
        let Some(h) = href.captures(t).map(|c| c[1].trim().to_string()) else {
            continue;
        };
        // Prefer a plain icon, then the Apple touch icon (larger, but always an image).
        let rank = if r.split_whitespace().any(|w| w == "icon") {
            2
        } else if r.contains("apple-touch-icon") {
            1
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| rank > *b) {
            best = Some((rank, h));
        }
    }
    best.map(|(_, h)| h.replace("&amp;", "&"))
}

async fn fetch_image(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = response.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return None;
    }
    let mime = image_type(&bytes, &content_type)?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

async fn fetch(host: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Lumo")
        .build()
        .ok()?;
    if let Some(icon) = fetch_image(&client, &format!("https://{host}/favicon.ico")).await {
        return Some(icon);
    }
    let base = url::Url::parse(&format!("https://{host}/")).ok()?;
    let response = client.get(base.clone()).send().await.ok()?;
    let page_url = response.url().clone();
    let bytes = response.bytes().await.ok()?;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PAGE_BYTES)]).into_owned();
    let link = icon_link(&html)?;
    let target = page_url.join(&link).ok()?;
    if target.scheme() != "https" && target.scheme() != "http" {
        return None;
    }
    fetch_image(&client, target.as_str()).await
}

/// The site's icon as a `data:` URL, from the cache or the site; `None` when it has none.
pub async fn get(config_dir: &Path, host: &str) -> Option<String> {
    let host = host.trim().trim_start_matches("www.").to_ascii_lowercase();
    if !valid_host(&host) {
        return None;
    }
    let path = cache_path(config_dir, &host);
    if let Ok(meta) = std::fs::metadata(&path) {
        let cached = crate::chat::at_rest::read_to_string(&path).unwrap_or_default();
        if !cached.is_empty() {
            return Some(cached);
        }
        let fresh = meta
            .modified()
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < MISS_TTL);
        if fresh {
            return None;
        }
    }
    // The bare domain first; many sites answer only on www.
    let icon = match fetch(&host).await {
        Some(icon) => Some(icon),
        None => fetch(&format!("www.{host}")).await,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::chat::at_rest::write(&path, icon.as_deref().unwrap_or_default());
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_links_and_types() {
        assert!(valid_host("apnews.com") && valid_host("www.aljazeera.com"));
        assert!(
            !valid_host("localhost")
                && !valid_host("a/b.com")
                && !valid_host("../x.com")
                && !valid_host("")
        );
        let html = r#"<head><link rel="stylesheet" href="/a.css"><link rel="apple-touch-icon" href="/touch.png">
            <link href="/static/fav.svg?v=2&amp;x=1" rel="shortcut icon"></head>"#;
        assert_eq!(icon_link(html).as_deref(), Some("/static/fav.svg?v=2&x=1"));
        assert_eq!(
            icon_link(r#"<link rel="apple-touch-icon" href="/t.png">"#).as_deref(),
            Some("/t.png")
        );
        assert!(icon_link("<p>no icon</p>").is_none());
        assert_eq!(
            image_type(&[0x89, b'P', b'N', b'G', 1], ""),
            Some("image/png")
        );
        assert_eq!(image_type(&[0, 0, 1, 0, 1], ""), Some("image/x-icon"));
        assert_eq!(image_type(b"<?xml?><svg xmlns=", ""), Some("image/svg+xml"));
        assert_eq!(image_type(b"<html>not found</html>", "text/html"), None);
    }

    #[test]
    fn a_site_without_an_icon_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path(), "example.com");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::chat::at_rest::write(&path, "").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // A fresh miss answers from the cache without going to the network.
        assert_eq!(rt.block_on(get(dir.path(), "example.com")), None);
        crate::chat::at_rest::write(&path, "data:image/png;base64,AAAA").unwrap();
        assert_eq!(
            rt.block_on(get(dir.path(), "www.EXAMPLE.com")).as_deref(),
            Some("data:image/png;base64,AAAA")
        );
    }
}
