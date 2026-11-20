//! Canvases: documents the assistant writes and the user edits beside the conversation.
//!
//! A canvas belongs to one conversation and keeps every version, so either side can change it
//! and earlier drafts stay recoverable. The model creates and rewrites canvases with the
//! `canvas` tool; the current text of each rides in the system prompt, so it sees the user's
//! own edits on the next turn.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::chat::store::{chat_dir, valid_id, write_json};

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
    script-src cortexapp: http://cortexapp.localhost 'unsafe-inline' 'unsafe-eval' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com; \
    style-src cortexapp: http://cortexapp.localhost 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://fonts.googleapis.com; \
    font-src cortexapp: http://cortexapp.localhost https://fonts.gstatic.com data:; img-src cortexapp: http://cortexapp.localhost data: blob: https:; media-src cortexapp: http://cortexapp.localhost data: blob: https:; \
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

// The app runs in a sandboxed frame with an opaque origin, where 'self' matches nothing; its
// own files are allowed by the app scheme instead.

/// A mini app made of several files (Vibe's apps with a file list): kept in the canvas as JSON
/// `{"lumoAppFiles": [{"path": "index.html", "content": "…"}, …]}`. One-file apps stay HTML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppFile {
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    #[serde(rename = "lumoAppFiles")]
    files: Vec<AppFile>,
}

/// The files of a multi-file app, or `None` for a one-file app.
pub fn app_files(content: &str) -> Option<Vec<AppFile>> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Bundle>(trimmed)
        .ok()
        .map(|b| b.files)
        .filter(|f| !f.is_empty())
}

/// Stores files as an app's content, checking their paths: relative, no `..`, at most 40 files.
pub fn bundle(files: &[AppFile]) -> Result<String, String> {
    if files.is_empty() || files.len() > 40 {
        return Err("An app has 1 to 40 files.".into());
    }
    for f in files {
        let p = f.path.trim();
        if p.is_empty()
            || p.starts_with('/')
            || p.contains('\\')
            || p.split('/')
                .any(|s| s == ".." || s.is_empty() || s.starts_with('.'))
        {
            return Err(format!(
                "`{}` is not a usable file path; use relative paths like css/app.css.",
                f.path
            ));
        }
    }
    if !files.iter().any(|f| f.path.ends_with(".html")) {
        return Err("An app needs an HTML file, usually index.html.".into());
    }
    serde_json::to_string(&Bundle {
        files: files.to_vec(),
    })
    .map_err(|e| e.to_string())
}

/// The page an app opens on: `index.html`, else its first HTML file.
fn entry(files: &[AppFile]) -> Option<&AppFile> {
    files
        .iter()
        .find(|f| f.path == "index.html")
        .or_else(|| files.iter().find(|f| f.path.ends_with(".html")))
}

fn mime(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "txt" | "md" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// An app's files as a ZIP archive, to download.
pub fn app_zip(content: &str) -> Result<Vec<u8>, String> {
    let files = app_files(content).unwrap_or_else(|| {
        vec![AppFile {
            path: "index.html".into(),
            content: app_html(content),
        }]
    });
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for f in &files {
            zip.start_file(f.path.as_str(), zip::write::SimpleFileOptions::default())
                .map_err(|e| e.to_string())?;
            std::io::Write::write_all(&mut zip, f.content.as_bytes()).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// An app's HTML: its content, unwrapped from a ```html fence if the model added one.
pub fn app_html(content: &str) -> String {
    let trimmed = content.trim();
    let inner = trimmed
        .strip_prefix("```html")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .map_or(trimmed, str::trim);
    if inner.to_ascii_lowercase().contains("<html")
        || inner.to_ascii_lowercase().starts_with("<!doctype")
    {
        inner.to_string()
    } else {
        format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head><body>{inner}</body></html>")
    }
}

/// The page of an app's own window: the app in the same sandboxed frame as inside Cortex, so it
/// gets no more access in a window than it has in a chat.
pub(in crate::chat::toolkit) fn window_html(id: &str, title: &str, windows: bool) -> String {
    let base = if windows {
        "http://cortexapp.localhost"
    } else {
        "cortexapp://localhost"
    };
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>html,body{{margin:0;height:100%;background:#fff}}iframe{{border:0;width:100%;height:100%;display:block}}</style></head>\
         <body><iframe sandbox=\"allow-scripts allow-forms allow-modals allow-popups\" src=\"{base}/{id}/\"></iframe></body></html>"
    )
}

/// The policy of an app window's page: only the app's frame.
const WINDOW_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; frame-src cortexapp: http://cortexapp.localhost";

/// Serves `cortexapp://localhost/<canvas id>` to the sandboxed preview frame, and
/// `cortexapp://localhost/window/<canvas id>` to an app's own window.
pub fn serve(config_dir: &Path, path: &str) -> tauri::http::Response<Vec<u8>> {
    let builder = tauri::http::Response::builder();
    if let Some(id) = path.trim_start_matches('/').strip_prefix("window/") {
        let id = id.split(['?', '/', '#']).next().unwrap_or_default();
        return match load(config_dir, id) {
            Ok(canvas) if canvas.kind == "app" => builder
                .status(200)
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Content-Security-Policy", WINDOW_CSP)
                .header("Cache-Control", "no-store")
                .body(window_html(&canvas.id, &canvas.title, cfg!(windows)).into_bytes())
                .unwrap_or_default(),
            _ => builder
                .status(404)
                .header("Content-Type", "text/plain")
                .body(b"No such app".to_vec())
                .unwrap_or_default(),
        };
    }
    let id = path
        .trim_start_matches('/')
        .split(['?', '/', '#'])
        .next()
        .unwrap_or_default();
    if id == DIAGRAM_PAGE {
        // Mermaid diagrams from answers, drawn in the same sandbox as mini apps.
        return builder
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Content-Security-Policy", APP_CSP)
            .body(DIAGRAM_HTML.as_bytes().to_vec())
            .unwrap_or_default();
    }
    // `/<id>/` is the app's page; `/<id>/<file>` one of its files.
    let file = path
        .trim_start_matches('/')
        .split_once('/')
        .map(|(_, rest)| rest.split(['?', '#']).next().unwrap_or_default())
        .unwrap_or_default();
    match load(config_dir, id) {
        Ok(canvas) if canvas.kind == "app" => {
            let (body, kind) = match app_files(canvas.content()) {
                Some(files) => {
                    let found = if file.is_empty() {
                        entry(&files)
                    } else {
                        files.iter().find(|f| f.path == file)
                    };
                    match found {
                        Some(f) => (f.content.clone().into_bytes(), mime(&f.path)),
                        None => {
                            return builder
                                .status(404)
                                .header("Content-Type", "text/plain")
                                .body(b"No such file in this app".to_vec())
                                .unwrap_or_default()
                        }
                    }
                }
                None if file.is_empty() || file == "index.html" => (
                    app_html(canvas.content()).into_bytes(),
                    "text/html; charset=utf-8",
                ),
                None => {
                    return builder
                        .status(404)
                        .header("Content-Type", "text/plain")
                        .body(b"No such file in this app".to_vec())
                        .unwrap_or_default()
                }
            };
            builder
                .status(200)
                .header("Content-Type", kind)
                .header("Content-Security-Policy", APP_CSP)
                .header("Cache-Control", "no-store")
                .body(body)
                .unwrap_or_default()
        }
        _ => builder
            .status(404)
            .header("Content-Type", "text/plain")
            .body(b"No such app".to_vec())
            .unwrap_or_default(),
    }
}

/// Every app canvas, newest first.
pub fn list_apps(config_dir: &Path) -> Vec<Canvas> {
    list_all(config_dir)
        .into_iter()
        .filter(|c| c.kind == "app")
        .collect()
}

/// Every canvas, newest change first.
pub fn list_all(config_dir: &Path) -> Vec<Canvas> {
    let Ok(entries) = std::fs::read_dir(dir(config_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<Canvas> = entries
        .filter_map(Result::ok)
        .filter_map(|e| crate::chat::at_rest::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Canvas>(&t).ok())
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
    let text = crate::chat::at_rest::read_to_string(path(config_dir, id)?)
        .map_err(|_| format!("no canvas `{id}`"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn list_for(config_dir: &Path, conversation_id: &str) -> Vec<Canvas> {
    let Ok(entries) = std::fs::read_dir(dir(config_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<Canvas> = entries
        .filter_map(Result::ok)
        .filter_map(|e| crate::chat::at_rest::read_to_string(e.path()).ok())
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
    write_kind(
        config_dir,
        conversation_id,
        id,
        title,
        content,
        author,
        None,
    )
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
        canvas.title = crate::support::text::clip(title, 120);
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
        canvas.versions.push(Version {
            content: content.to_string(),
            author: author.to_string(),
            at: Utc::now(),
        });
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
        let content: String = crate::support::text::clip(c.content(), PROMPT_CHARS);
        out.push_str(&format!(
            "\n\n<canvas id=\"{}\" kind=\"{}\" title=\"{}\">\n{}\n</canvas>",
            c.id, c.kind, c.title, content
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_served_with_their_own_policy() {
        let dir = tempfile::tempdir().unwrap();
        let app = write_kind(
            dir.path(),
            "c",
            None,
            Some("Timer"),
            "```html\n<!doctype html><html><body><script>1</script></body></html>\n```",
            "assistant",
            Some("app"),
        )
        .unwrap();
        let doc = write(dir.path(), "c", None, None, "# Notes", "assistant").unwrap();
        let ok = serve(dir.path(), &format!("/{}", app.id));
        assert_eq!(ok.status(), 200);
        assert!(String::from_utf8_lossy(ok.body()).starts_with("<!doctype html>"));
        assert!(ok.headers()["Content-Security-Policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'"));
        assert_eq!(serve(dir.path(), &format!("/{}", doc.id)).status(), 404);
        assert_eq!(serve(dir.path(), "/../x").status(), 404);
        assert_eq!(list_apps(dir.path()).len(), 1);
        assert!(app_html("<p>hi</p>").contains("<body><p>hi</p></body>"));
    }

    #[test]
    fn canvases_keep_versions_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let c = write(
            dir.path(),
            "conv",
            None,
            None,
            "# Plan\nStep one",
            "assistant",
        )
        .unwrap();
        assert_eq!(c.title, "Plan");
        let c2 = write(
            dir.path(),
            "conv",
            Some(&c.id),
            Some("Trip plan"),
            "# Plan\nStep two",
            "user",
        )
        .unwrap();
        assert_eq!(c2.title, "Trip plan");
        assert_eq!(c2.versions.len(), 2);
        assert_eq!(c2.content(), "# Plan\nStep two");
        // Unchanged content adds no version.
        let c3 = write(
            dir.path(),
            "conv",
            Some(&c.id),
            None,
            "# Plan\nStep two",
            "user",
        )
        .unwrap();
        assert_eq!(c3.versions.len(), 2);
        assert!(write(dir.path(), "other", Some(&c.id), None, "x", "user").is_err());
        assert_eq!(list_for(dir.path(), "conv").len(), 1);
        assert!(prompt(&list_for(dir.path(), "conv"))
            .unwrap()
            .contains("Step two"));
        assert!(prompt(&[]).is_none());
    }

    #[test]
    fn an_app_window_frames_the_app_in_the_sandbox() {
        let page = window_html("abc", "Tip <calc>", false);
        assert!(page.contains(r#"sandbox="allow-scripts allow-forms allow-modals allow-popups""#));
        assert!(page.contains(r#"src="cortexapp://localhost/abc/""#));
        assert!(page.contains("<title>Tip &lt;calc&gt;</title>"));
        assert!(!page.contains("allow-same-origin"));
        assert!(window_html("abc", "x", true).contains("http://cortexapp.localhost/abc/"));
    }

    #[test]
    fn apps_can_have_several_files() {
        let files = vec![
            AppFile {
                path: "index.html".into(),
                content: "<link rel=stylesheet href=style.css><script src=js/app.js></script>"
                    .into(),
            },
            AppFile {
                path: "style.css".into(),
                content: "body{color:red}".into(),
            },
            AppFile {
                path: "js/app.js".into(),
                content: "console.log(1)".into(),
            },
        ];
        let content = bundle(&files).unwrap();
        assert_eq!(app_files(&content).unwrap(), files);
        assert_eq!(entry(&files).unwrap().path, "index.html");
        assert_eq!(mime("js/app.js"), "text/javascript; charset=utf-8");
        assert!(app_files("<!doctype html><p>one file</p>").is_none());
        assert!(bundle(&[AppFile {
            path: "../x.html".into(),
            content: String::new()
        }])
        .is_err());
        assert!(bundle(&[AppFile {
            path: "app.js".into(),
            content: String::new()
        }])
        .is_err());
        // The ZIP holds every file; a one-file app downloads as index.html.
        let zip = app_zip(&content).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        assert_eq!(archive.len(), 3);
        assert!(archive.by_name("js/app.js").is_ok());
        let single = app_zip("<p>hi</p>").unwrap();
        assert!(zip::ZipArchive::new(std::io::Cursor::new(single))
            .unwrap()
            .by_name("index.html")
            .is_ok());
    }
}
