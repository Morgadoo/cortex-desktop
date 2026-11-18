//! Canvases: documents the assistant writes and the user edits beside the conversation.
//!
//! A canvas belongs to one conversation and keeps every version, so either side can change it
//! and earlier drafts stay recoverable. The model creates and rewrites canvases with the
//! `canvas` tool; the current text of each rides in the system prompt, so it sees the user's
//! own edits on the next turn.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub content: String,
    /// `assistant` or `user`.
    pub author: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Canvas {
    pub id: String,
    pub conversation_id: String,
    pub title: String,
    /// `document` (Markdown), `app` (a self-contained HTML page run in a sandbox) or `sheet`
    /// (a table, as CSV).
    #[serde(default = "document_kind")]
    pub kind: String,
    pub versions: Vec<Version>,
}

impl Canvas {
    pub fn content(&self) -> &str {
        self.versions.last().map_or("", |v| v.content.as_str())
    }
}

fn document_kind() -> String {
    "document".into()
}

/// What may run inside an app: inline code, and libraries from the common CDNs only. The app has
/// no access to the chat's IPC; the iframe sandbox gives it a unique origin.
pub const APP_CSP: &str = "default-src 'none'; \
    script-src 'unsafe-inline' 'unsafe-eval' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com; \
    style-src 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://fonts.googleapis.com; \
    font-src https://fonts.gstatic.com data:; img-src data: blob: https:; media-src data: blob: https:; \
    connect-src https:; form-action 'none'; base-uri 'none'";

/// The address, under the app protocol, of the page that draws Mermaid diagrams.
pub const DIAGRAM_PAGE: &str = "_diagram";

/// Draws the Mermaid code (or Marp slides) in its URL's fragment with Mermaid from the CDN mini apps may use, and
/// tells the conversation its height. Runs sandboxed, with no access to the app.
const DIAGRAM_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<style>html,body{margin:0;background:transparent;font-family:system-ui,sans-serif}#out{display:flex;justify-content:center;padding:8px}#out svg{max-width:100%;height:auto}.err{color:#e5866d;font:12px ui-monospace,monospace;white-space:pre-wrap;padding:8px}</style>
</head><body><div id="out"></div>
<script type="module">
const params = new URLSearchParams(location.hash.slice(1));
const key = params.get('key') || '';
// Paint the card's colour, so the frame blends with the conversation in either theme.
const bg = params.get('bg') || '';
if (/^(#[0-9a-f]{3,8}|rgba?\([\d\s.,%]+\))$/i.test(bg)) document.documentElement.style.background = document.body.style.background = bg;
document.documentElement.style.colorScheme = params.get('theme') === 'light' ? 'light' : 'dark';
const out = document.getElementById('out');
const report = () => parent.postMessage({ diagramKey: key, height: Math.ceil(document.documentElement.scrollHeight) }, '*');
try {
  if (params.get('kind') === 'marp') {
    // A Marp deck: every slide, scaled to the card's width, one under the other.
    const { Marp } = await import('https://cdn.jsdelivr.net/npm/@marp-team/marp-core@4.0.0/+esm');
    const { html, css } = new Marp({ html: false, math: false }).render(params.get('code') || '');
    const style = document.createElement('style');
    style.textContent = css + ' .marpit{width:100%} .marpit > svg{display:block;width:100%;height:auto;margin:0 0 10px;border-radius:6px;box-shadow:0 1px 4px rgba(0,0,0,.35)}';
    document.head.append(style);
    out.style.display = 'block';
    out.innerHTML = html;
  } else {
    const { default: mermaid } = await import('https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs');
    mermaid.initialize({ startOnLoad: false, theme: params.get('theme') === 'light' ? 'default' : 'dark', securityLevel: 'strict' });
    const { svg } = await mermaid.render('diagram', params.get('code') || '');
    out.innerHTML = svg;
  }
} catch (e) {
  const div = document.createElement('div');
  div.className = 'err';
  div.textContent = 'This diagram could not be drawn: ' + (e && e.message ? e.message : String(e));
  out.replaceChildren(div);
}
report();
new ResizeObserver(report).observe(document.body);
// The page can ask for the drawn SVG, to save it as a file.
window.addEventListener('message', (e) => {
  if (!e.data || e.data.exportKey !== key) return;
  const svg = out.querySelector('svg');
  parent.postMessage({ diagramKey: key, svg: svg ? new XMLSerializer().serializeToString(svg) : null }, '*');
});
</script></body></html>"#;

/// An app's HTML: its content, unwrapped from a ```html fence if the model added one.
pub fn app_html(content: &str) -> String {
    let trimmed = content.trim();
    let inner = trimmed
        .strip_prefix("```html")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .map_or(trimmed, str::trim);
    if inner.to_ascii_lowercase().contains("<html") || inner.to_ascii_lowercase().starts_with("<!doctype") {
        inner.to_string()
    } else {
        format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head><body>{inner}</body></html>")
    }
}

/// Serves `lumoapp://localhost/<canvas id>` to the sandboxed preview frame.
pub fn serve(config_dir: &Path, path: &str) -> tauri::http::Response<Vec<u8>> {
    let id = path.trim_start_matches('/').split(['?', '/', '#']).next().unwrap_or_default();
    let builder = tauri::http::Response::builder();
    if id == DIAGRAM_PAGE {
        // Mermaid diagrams from answers, drawn in the same sandbox as mini apps.
        return builder
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Content-Security-Policy", APP_CSP)
            .body(DIAGRAM_HTML.as_bytes().to_vec())
            .unwrap_or_default();
    }
    match load(config_dir, id) {
        Ok(canvas) if canvas.kind == "app" => builder
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Content-Security-Policy", APP_CSP)
            .header("Cache-Control", "no-store")
            .body(app_html(canvas.content()).into_bytes())
            .unwrap_or_default(),
        _ => builder.status(404).header("Content-Type", "text/plain").body(b"No such app".to_vec()).unwrap_or_default(),
    }
}

/// Every app canvas, newest first.
pub fn list_apps(config_dir: &Path) -> Vec<Canvas> {
    let Ok(entries) = std::fs::read_dir(dir(config_dir)) else { return Vec::new() };
    let mut out: Vec<Canvas> = entries
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Canvas>(&t).ok())
        .filter(|c| c.kind == "app")
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.versions.last().map(|v| v.at)));
    out
}

/// Characters of each canvas shown to the model in the system prompt.
pub const PROMPT_CHARS: usize = 12_000;
/// Versions kept per canvas.
pub const MAX_VERSIONS: usize = 50;

fn dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("canvases")
}

fn path(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("invalid canvas id `{id}`"));
    }
    Ok(dir(config_dir).join(format!("{id}.json")))
}

pub fn load(config_dir: &Path, id: &str) -> Result<Canvas, String> {
    let text = std::fs::read_to_string(path(config_dir, id)?).map_err(|_| format!("no canvas `{id}`"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn list_for(config_dir: &Path, conversation_id: &str) -> Vec<Canvas> {
    let Ok(entries) = std::fs::read_dir(dir(config_dir)) else { return Vec::new() };
    let mut out: Vec<Canvas> = entries
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Canvas>(&t).ok())
        .filter(|c| c.conversation_id == conversation_id)
        .collect();
    out.sort_by_key(|c| c.versions.first().map(|v| v.at));
    out
}

/// Writes a new version, creating the canvas when `id` is `None`.
pub fn write(
    config_dir: &Path,
    conversation_id: &str,
    id: Option<&str>,
    title: Option<&str>,
    content: &str,
    author: &str,
) -> Result<Canvas, String> {
    write_kind(config_dir, conversation_id, id, title, content, author, None)
}

/// [`write`], setting the kind of a new canvas (an existing one keeps its own).
pub fn write_kind(
    config_dir: &Path,
    conversation_id: &str,
    id: Option<&str>,
    title: Option<&str>,
    content: &str,
    author: &str,
    kind: Option<&str>,
) -> Result<Canvas, String> {
    let mut canvas = match id {
        Some(id) => load(config_dir, id)?,
        None => Canvas {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            title: String::new(),
            kind: match kind {
                Some("app") => "app".into(),
                Some("sheet") => "sheet".into(),
                _ => "document".into(),
            },
            versions: Vec::new(),
        },
    };
    if canvas.conversation_id != conversation_id {
        return Err("That canvas belongs to another conversation.".into());
    }
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        canvas.title = title.chars().take(120).collect();
    }
    if canvas.title.is_empty() {
        canvas.title = content
            .lines()
            .map(|l| l.trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("Untitled")
            .chars()
            .take(80)
            .collect();
    }
    if canvas.content() != content {
        canvas.versions.push(Version { content: content.to_string(), author: author.to_string(), at: Utc::now() });
        if canvas.versions.len() > MAX_VERSIONS {
            canvas.versions.remove(0);
        }
    }
    write_json(&path(config_dir, &canvas.id)?, &canvas)?;
    Ok(canvas)
}

/// The system-prompt section listing a conversation's canvases with their current text.
pub fn prompt(canvases: &[Canvas]) -> Option<String> {
    if canvases.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\nCanvases in this conversation (current text, including the user's own edits; update \
         one with `canvas` and its id):",
    );
    for c in canvases {
        let content: String = c.content().chars().take(PROMPT_CHARS).collect();
        out.push_str(&format!("\n\n<canvas id=\"{}\" kind=\"{}\" title=\"{}\">\n{}\n</canvas>", c.id, c.kind, c.title, content));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_served_with_their_own_policy() {
        let dir = tempfile::tempdir().unwrap();
        let app = write_kind(dir.path(), "c", None, Some("Timer"), "```html\n<!doctype html><html><body><script>1</script></body></html>\n```", "assistant", Some("app")).unwrap();
        let doc = write(dir.path(), "c", None, None, "# Notes", "assistant").unwrap();
        let ok = serve(dir.path(), &format!("/{}", app.id));
        assert_eq!(ok.status(), 200);
        assert!(String::from_utf8_lossy(ok.body()).starts_with("<!doctype html>"));
        assert!(ok.headers()["Content-Security-Policy"].to_str().unwrap().contains("default-src 'none'"));
        assert_eq!(serve(dir.path(), &format!("/{}", doc.id)).status(), 404);
        assert_eq!(serve(dir.path(), "/../x").status(), 404);
        assert_eq!(list_apps(dir.path()).len(), 1);
        assert!(app_html("<p>hi</p>").contains("<body><p>hi</p></body>"));
    }

    #[test]
    fn canvases_keep_versions_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let c = write(dir.path(), "conv", None, None, "# Plan\nStep one", "assistant").unwrap();
        assert_eq!(c.title, "Plan");
        let c2 = write(dir.path(), "conv", Some(&c.id), Some("Trip plan"), "# Plan\nStep two", "user").unwrap();
        assert_eq!(c2.title, "Trip plan");
        assert_eq!(c2.versions.len(), 2);
        assert_eq!(c2.content(), "# Plan\nStep two");
        // Unchanged content adds no version.
        let c3 = write(dir.path(), "conv", Some(&c.id), None, "# Plan\nStep two", "user").unwrap();
        assert_eq!(c3.versions.len(), 2);
        assert!(write(dir.path(), "other", Some(&c.id), None, "x", "user").is_err());
        assert_eq!(list_for(dir.path(), "conv").len(), 1);
        assert!(prompt(&list_for(dir.path(), "conv")).unwrap().contains("Step two"));
        assert!(prompt(&[]).is_none());
    }
}
