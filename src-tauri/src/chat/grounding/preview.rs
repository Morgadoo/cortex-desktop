//! Link previews for the hover card on a citation, as Vibe shows them: the page's title, its
//! description, and its preview image.
//!
//! Read from the page's Open Graph tags (or its `<title>` and description), with the image
//! fetched by the app and handed to the page as a `data:` URL, since the page may only load
//! images from itself. Kept in `chat/previews/` so a source is fetched once.

use crate::chat::error::ChatError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::chat::store::{chat_dir, write_json};

const MAX_PAGE_BYTES: usize = 512 * 1024;
const MAX_IMAGE_BYTES: usize = 1_500 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub url: String,
    pub site: String,
    pub title: String,
    pub description: String,
    /// A `data:` URL, when the page has a usable preview image.
    pub image: Option<String>,
    pub published: Option<String>,
}

fn cache_path(config_dir: &Path, url: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    chat_dir(config_dir)
        .join("previews")
        .join(format!("{:016x}.json", hasher.finish()))
}

fn decode(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/// The content of `<meta property|name="key" content="…">`, in either attribute order.
fn meta(html: &str, keys: &[&str]) -> Option<String> {
    let tag = regex::Regex::new(r"(?is)<meta\b[^>]*>").ok()?;
    let attr = |t: &str, name: &str| -> Option<String> {
        let re =
            regex::Regex::new(&format!(r#"(?is)\b{name}\s*=\s*(?:"([^"]*)"|'([^']*)')"#)).ok()?;
        re.captures(t)
            .and_then(|c| c.get(1).or(c.get(2)))
            .map(|m| m.as_str().to_string())
    };
    for key in keys {
        for m in tag.find_iter(html) {
            let t = m.as_str();
            let named = attr(t, "property")
                .or_else(|| attr(t, "name"))
                .map(|v| v.to_ascii_lowercase());
            if named.as_deref() == Some(*key) {
                if let Some(content) = attr(t, "content")
                    .map(|c| decode(c.trim()))
                    .filter(|c| !c.is_empty())
                {
                    return Some(content);
                }
            }
        }
    }
    None
}

/// Title, description, image URL and date from a page's HTML.
pub(in crate::chat::grounding) fn parse(
    html: &str,
) -> (String, String, Option<String>, Option<String>) {
    let title = meta(html, &["og:title", "twitter:title"]).or_else(|| {
        regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>")
            .ok()?
            .captures(html)
            .map(|c| decode(c[1].trim()))
    });
    let description = meta(
        html,
        &["og:description", "twitter:description", "description"],
    );
    let image = meta(
        html,
        &[
            "og:image",
            "og:image:url",
            "twitter:image",
            "twitter:image:src",
        ],
    );
    let published = meta(
        html,
        &[
            "article:published_time",
            "og:published_time",
            "date",
            "pubdate",
        ],
    );
    (
        title
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        description
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        image,
        published,
    )
}

async fn fetch_image(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = response.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mime = if mime.starts_with("image/") && !mime.contains("svg") {
        mime.split(';').next().unwrap_or("image/jpeg").to_string()
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg".into()
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png".into()
    } else if bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp".into()
    } else {
        return None;
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// The preview of a web page, from the cache or the page.
pub async fn get(config_dir: &Path, url: &str) -> Result<Preview, ChatError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| "Not a web address.".to_string())
        .map_err(ChatError::Invalid)?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(ChatError::Invalid("Not a web address.".into()));
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string();
    if !crate::chat::favicon::valid_host(&host) {
        return Err(ChatError::Invalid("Not a public web address.".into()));
    }
    let path = cache_path(config_dir, url);
    if let Some(cached) = crate::chat::at_rest::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Preview>(&t).ok())
    {
        return Ok(cached);
    }
    let client =
        crate::support::http::browser_client(Duration::from_secs(8)).map_err(ChatError::Invalid)?;
    let response = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(ChatError::internal)?;
    let page_url = response.url().clone();
    let bytes = response.bytes().await.map_err(ChatError::internal)?;
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PAGE_BYTES)]).into_owned();
    let (title, description, image, published) = parse(&html);
    let image = match image.and_then(|i| page_url.join(&i).ok()) {
        Some(src) if src.scheme() == "https" || src.scheme() == "http" => {
            fetch_image(&client, src.as_str()).await
        }
        _ => None,
    };
    let preview = Preview {
        url: url.to_string(),
        site: host,
        title,
        description,
        image,
        published,
    };
    let _ = write_json(&path, &preview);
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_graph_and_fallbacks() {
        let html = r#"<html><head><title>Fallback title</title>
            <meta content="Room rents rise in Portugal - The Portugal News" property="og:title">
            <meta name="description" content="Renting a room has become more expensive &amp; scarce.">
            <meta property="og:image" content='/img/rent.jpg'>
            <meta property="article:published_time" content="2026-11-14T10:00:00Z"></head></html>"#;
        let (title, description, image, published) = parse(html);
        assert_eq!(title, "Room rents rise in Portugal - The Portugal News");
        assert_eq!(
            description,
            "Renting a room has become more expensive & scarce."
        );
        assert_eq!(image.as_deref(), Some("/img/rent.jpg"));
        assert_eq!(published.as_deref(), Some("2026-11-14T10:00:00Z"));
        let (title, description, image, _) = parse("<title>\n Only a title </title>");
        assert_eq!(
            (title.as_str(), description.as_str(), image),
            ("Only a title", "", None)
        );
    }

    #[test]
    fn refuses_what_is_not_a_public_page() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(rt.block_on(get(dir.path(), "file:///etc/passwd")).is_err());
        assert!(rt
            .block_on(get(dir.path(), "http://localhost:8080/"))
            .is_err());
        assert!(rt.block_on(get(dir.path(), "not a url")).is_err());
    }
}
