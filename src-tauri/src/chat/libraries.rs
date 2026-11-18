//! Libraries: document collections the assistant searches when answering.
//!
//! Local-first retrieval. Documents are extracted to text once (PDF through rig's loader, HTML
//! through `scraper`, everything else as text) and stored under `<config>/chat/libraries/<id>/`.
//! Search is BM25 over overlapping passages, so it needs no embedding model and works with any
//! provider; with an embedding model set up, `embeddings` adds semantic search and fuses both; the `search_library` tool hands the model the best passages with their document
//! names, which it cites.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub name: String,
    pub chars: usize,
    pub added_at: DateTime<Utc>,
    /// For a document mirrored from a folder: the file, and when it last changed (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub documents: Vec<Document>,
    /// Folders the library mirrors: their files are added, updated and removed as they change.
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    pub library: String,
    pub document: String,
    pub passage: usize,
    pub text: String,
    pub score: f64,
}

/// Characters per passage, and how much consecutive passages share.
const PASSAGE: usize = 1_200;
const OVERLAP: usize = 200;
/// The largest document accepted, in extracted characters.
const MAX_DOC_CHARS: usize = 5_000_000;

fn root(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("libraries")
}

fn lib_dir(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("invalid library id".into());
    }
    Ok(root(config_dir).join(id))
}

pub fn list(config_dir: &Path) -> Vec<Library> {
    let Ok(entries) = std::fs::read_dir(root(config_dir)) else { return Vec::new() };
    let mut out: Vec<Library> = entries
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_to_string(e.path().join("library.json")).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn load(config_dir: &Path, id: &str) -> Result<Library, String> {
    let text = std::fs::read_to_string(lib_dir(config_dir, id)?.join("library.json")).map_err(|_| "No such library.".to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn save(config_dir: &Path, library: &Library) -> Result<(), String> {
    write_json(&lib_dir(config_dir, &library.id)?.join("library.json"), library)
}

pub fn create(config_dir: &Path, name: &str, description: &str) -> Result<Library, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A library needs a name.".into());
    }
    let library = Library {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.chars().take(80).collect(),
        description: description.trim().to_string(),
        created_at: Utc::now(),
        documents: Vec::new(),
        folders: Vec::new(),
        synced_at: None,
    };
    save(config_dir, &library)?;
    Ok(library)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<(), String> {
    let dir = lib_dir(config_dir, id)?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Office documents (Word, PowerPoint, OpenDocument text and slides) by their zipped XML.
pub const OFFICE_EXTENSIONS: &[&str] = &["docx", "pptx", "odt", "odp"];

/// The text of an Office document: paragraphs on their own lines, slides in order.
pub fn office_text(bytes: &[u8], ext: &str) -> Result<String, String> {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "The document could not be opened (not an Office file?).".to_string())?;
    let mut parts: Vec<String> = match ext {
        "docx" => {
            let mut names = vec!["word/document.xml".to_string()];
            names.extend(zip.file_names().filter(|n| n.starts_with("word/footnotes") && n.ends_with(".xml")).map(str::to_string));
            names
        }
        "pptx" => {
            let mut slides: Vec<(u32, String)> = zip
                .file_names()
                .filter_map(|n| n.strip_prefix("ppt/slides/slide").and_then(|r| r.strip_suffix(".xml")).and_then(|d| d.parse().ok()).map(|d| (d, n.to_string())))
                .collect();
            slides.sort();
            slides.into_iter().map(|(_, n)| n).collect()
        }
        _ => vec!["content.xml".to_string()],
    };
    parts.retain(|n| zip.by_name(n).is_ok());
    if parts.is_empty() {
        return Err("The document has no text part.".into());
    }
    let mut out = Vec::new();
    for (i, name) in parts.iter().enumerate() {
        let mut xml = String::new();
        zip.by_name(name).map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
        let text = xml_text(&xml);
        if ext == "pptx" {
            out.push(format!("Slide {}:\n{}", i + 1, text.trim()));
        } else {
            out.push(text.trim().to_string());
        }
    }
    Ok(out.join("\n\n"))
}

/// Text from WordprocessingML, DrawingML or ODF XML: paragraph and line ends become newlines,
/// tabs become tabs, tags are dropped and entities decoded.
fn xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let between = &rest[..start];
        // Indentation between tags in pretty-printed XML is not text.
        if !(between.trim().is_empty() && between.contains('\n')) {
            out.push_str(&decode_entities(between));
        }
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start + 1..start + end];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default();
        let closing = tag.starts_with('/');
        match name {
            "w:p" | "a:p" | "text:p" | "text:h" if closing => out.push('\n'),
            "w:br" | "a:br" | "text:line-break" | "w:cr" => out.push('\n'),
            "w:tab" | "text:tab" if !closing => out.push('\t'),
            "text:s" if !closing => out.push(' '),
            "w:tc" | "a:tc" | "table:table-cell" if closing => out.push('\t'),
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    // Paragraph text only appears inside <w:t>/<a:t>/text:p, but stray whitespace between tags
    // in pretty-printed XML would add blank lines: keep at most one empty line.
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in out.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    cleaned
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The text files in a ZIP archive, each under its path, until `budget` characters (OpenBot's ZIP
/// attachments). Office documents inside are read too; binaries are listed but skipped.
pub fn zip_text(bytes: &[u8], budget: usize) -> Result<String, String> {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "That is not a ZIP archive.".to_string())?;
    let mut out = String::new();
    let mut skipped = Vec::new();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let mut names: Vec<String> = names.into_iter().filter(|n| !n.ends_with('/') && !n.starts_with("__MACOSX/") && !n.split('/').any(|p| p.starts_with('.'))).collect();
    names.sort();
    for name in &names {
        if out.len() >= budget {
            skipped.push(name.clone());
            continue;
        }
        let mut file = match zip.by_name(name) {
            Ok(f) if f.size() <= 20 * 1024 * 1024 => f,
            _ => {
                skipped.push(name.clone());
                continue;
            }
        };
        let mut data = Vec::new();
        if file.read_to_end(&mut data).is_err() {
            skipped.push(name.clone());
            continue;
        }
        let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        let text = if OFFICE_EXTENSIONS.contains(&ext.as_str()) {
            office_text(&data, &ext).ok()
        } else if data.iter().take(8000).any(|b| *b == 0) {
            None
        } else {
            Some(String::from_utf8_lossy(&data).into_owned())
        };
        match text {
            Some(t) if !t.trim().is_empty() => {
                let room = budget.saturating_sub(out.len());
                let t: String = t.chars().take(room).collect();
                out.push_str(&format!("--- {name} ---\n{}\n\n", t.trim_end()));
            }
            _ => skipped.push(name.clone()),
        }
    }
    if out.is_empty() {
        return Err("The archive has no readable text files.".into());
    }
    if !skipped.is_empty() {
        out.push_str(&format!("(Not read: {})", skipped.iter().take(30).cloned().collect::<Vec<_>>().join(", ")));
    }
    Ok(out)
}

/// Moves a document to another library, its text and name kept.
pub fn move_document(config_dir: &Path, from: &str, document_id: &str, to: &str) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    let source = load(config_dir, from)?;
    let doc = source.documents.iter().find(|d| d.id == document_id).ok_or("That document is gone.")?.clone();
    let text = std::fs::read_to_string(docs_dir(config_dir, from)?.join(format!("{}.txt", doc.id))).map_err(|e| e.to_string())?;
    load(config_dir, to)?;
    add_text(config_dir, to, &doc.name, &text)?;
    remove(config_dir, from, document_id).map(|_| ())
}

/// The text of a file with its line breaks, by its kind: PDF, Office, HTML or plain text.
pub fn read_text(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let text = match ext.as_str() {
        "pdf" => {
            let pattern = glob::Pattern::escape(&path.to_string_lossy());
            let first = rig_core::loaders::PdfFileLoader::with_glob(&pattern)
                .map_err(|e| e.to_string())?
                .read()
                .into_iter()
                .next();
            first
                .ok_or("The PDF could not be read.")?
                .map_err(|e| format!("The PDF could not be read: {e}"))?
        }
        e if OFFICE_EXTENSIONS.contains(&e) => office_text(&std::fs::read(path).map_err(|e| e.to_string())?, e)?,
        "html" | "htm" => {
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let doc = scraper::Html::parse_document(&raw);
            let body = scraper::Selector::parse("body").expect("selector");
            doc.select(&body)
                .next()
                .map(|b| b.text().collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
        }
        _ => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.iter().take(8000).any(|b| *b == 0) {
                return Err(format!("{} is not a text document (PDF, Word, PowerPoint, OpenDocument, HTML, Markdown, text, code and data files are supported).", path.display()));
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    Ok(text)
}

/// The text of a file, by its kind, with whitespace collapsed for indexing.
pub fn extract(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let text = read_text(path)?;
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        return Err(if ext == "pdf" {
            format!("{} has no text layer (a scan?). Add its pages as images instead; they are read by your model.", path.display())
        } else {
            format!("{} has no readable text.", path.display())
        });
    }
    if text.chars().count() > MAX_DOC_CHARS {
        return Err(format!("{} is too large to index.", path.display()));
    }
    Ok(text)
}

/// Adds a file's text to a library.
pub fn add(config_dir: &Path, library_id: &str, path: &Path) -> Result<Library, String> {
    let text = extract(path)?;
    let name = path.file_name().map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
    add_text(config_dir, library_id, &name, &text)
}

/// Image files, whose text is read by a model that can see (`ocr`).
pub fn is_image(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Adds text already extracted (from a file, or read from an image) as a document.
pub fn add_text(config_dir: &Path, library_id: &str, name: &str, text: &str) -> Result<Library, String> {
    let mut library = load(config_dir, library_id)?;
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        return Err(format!("{name} has no readable text."));
    }
    if text.chars().count() > MAX_DOC_CHARS {
        return Err(format!("{name} is too large to index."));
    }
    let doc = Document {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        chars: text.chars().count(),
        added_at: Utc::now(),
        source: None,
        modified: None,
    };
    let docs = lib_dir(config_dir, library_id)?.join("docs");
    std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    std::fs::write(docs.join(format!("{}.txt", doc.id)), text).map_err(|e| e.to_string())?;
    library.documents.push(doc);
    save(config_dir, &library)?;
    Ok(library)
}

/// Where a library keeps its documents' text (and their cached vectors).
pub fn docs_dir(config_dir: &Path, library_id: &str) -> Result<PathBuf, String> {
    Ok(lib_dir(config_dir, library_id)?.join("docs"))
}

pub fn remove(config_dir: &Path, library_id: &str, document_id: &str) -> Result<Library, String> {
    let mut library = load(config_dir, library_id)?;
    if !valid_id(document_id) {
        return Err("invalid document id".into());
    }
    library.documents.retain(|d| d.id != document_id);
    let docs = docs_dir(config_dir, library_id)?;
    let _ = std::fs::remove_file(docs.join(format!("{document_id}.txt")));
    // Its cached vectors, for every model.
    if let Ok(entries) = std::fs::read_dir(&docs) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{document_id}.")) && name.ends_with(".vec.json") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    save(config_dir, &library)?;
    Ok(library)
}

/// Characters of a document `read_document` returns per part.
const READ_PART: usize = 40_000;

/// A whole document (or one part of a long one), found by name, for `read_document`.
pub fn read_document(config_dir: &Path, library: &str, name: &str, part: usize) -> Result<String, String> {
    let wanted = name.trim().to_lowercase();
    let found = list(config_dir).into_iter().filter(|l| library.trim().is_empty() || l.name.eq_ignore_ascii_case(library.trim())).find_map(|l| {
        let doc = l
            .documents
            .iter()
            .find(|d| d.name.to_lowercase() == wanted)
            .or_else(|| l.documents.iter().find(|d| d.name.to_lowercase().contains(&wanted)))
            .cloned()?;
        Some((l, doc))
    });
    let Some((library, doc)) = found else {
        return Err(format!("No library document is named {name}; use a name from search_library."));
    };
    let text = std::fs::read_to_string(docs_dir(config_dir, &library.id)?.join(format!("{}.txt", doc.id))).map_err(|e| e.to_string())?;
    let chars: Vec<char> = text.chars().collect();
    let parts = chars.len().div_ceil(READ_PART).max(1);
    if part > parts {
        return Err(format!("{} has {parts} part(s).", doc.name));
    }
    let slice: String = chars[(part - 1) * READ_PART..(part * READ_PART).min(chars.len())].iter().collect();
    Ok(format!("# {} ({}), part {part} of {parts}\n\n{slice}", doc.name, library.name))
}

/// Saves a web page's readable text into a library (Onyx's web connector, Open WebUI's "add a
/// webpage"); the document is named after the page and starts with its address.
pub async fn add_web_page(config_dir: &Path, library_id: &str, url: &str) -> Result<Library, String> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| "That is not a web address.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https pages can be saved.".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Lumo")
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(parsed.clone()).send().await.map_err(|e| format!("The page could not be fetched: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("The page answered {}.", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("That page is too large to save.".into());
    }
    let html = String::from_utf8_lossy(&bytes);
    let text = crate::tools::connectors::browser::html_to_text(&html);
    if text.trim().len() < 40 {
        return Err("That page has no readable text (it may need JavaScript).".into());
    }
    let title = crate::tools::connectors::browser::extract_title(&html);
    let host = parsed.host_str().unwrap_or("page").trim_start_matches("www.").to_string();
    let name: String = if title.trim().is_empty() { format!("{host}{}", parsed.path()) } else { format!("{} ({host})", title.trim()) }.chars().take(120).collect();
    add_text(config_dir, library_id, &name, &format!("Source: {parsed}\n\n{text}"))
}

/// The indexed text of a document, for its preview (at most `max` characters).
pub fn document_text(config_dir: &Path, library_id: &str, document_id: &str, max: usize) -> Result<String, String> {
    let library = load(config_dir, library_id)?;
    let doc = library.documents.iter().find(|d| d.id == document_id).ok_or("That document is gone.")?;
    let text = std::fs::read_to_string(docs_dir(config_dir, &library.id)?.join(format!("{}.txt", doc.id))).map_err(|e| e.to_string())?;
    Ok(if text.chars().count() > max { format!("{}\n\n… (the rest is cut here; the assistant reads it all with read_document)", text.chars().take(max).collect::<String>()) } else { text })
}

/// File types a mirrored folder indexes.
const SYNC_EXTENSIONS: &[&str] = &["pdf", "docx", "pptx", "odt", "odp", "md", "markdown", "txt", "html", "htm", "csv", "json", "yaml", "yml", "xml", "rs", "py", "js", "ts", "go", "java", "c", "cpp", "h", "sql", "log", "rst", "org", "tex"];
/// Files indexed per folder, and how deep a folder is walked.
const SYNC_MAX_FILES: usize = 2_000;
const SYNC_MAX_DEPTH: usize = 6;

/// What a sync changed.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: Vec<String>,
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > SYNC_MAX_DEPTH || out.len() >= SYNC_MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hidden files and folders, and build and dependency folders, are not documents.
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "__pycache__" | "venv" | ".venv") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, depth + 1, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| SYNC_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())) {
            out.push(path);
            if out.len() >= SYNC_MAX_FILES {
                return;
            }
        }
    }
}

fn modified_secs(path: &Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
}

/// Brings a library's mirrored documents in line with its folders: new files are added, changed
/// ones re-read, and files that are gone (or folders no longer mirrored) leave the library.
pub fn sync(config_dir: &Path, library_id: &str) -> Result<SyncReport, String> {
    let mut library = load(config_dir, library_id)?;
    let docs = docs_dir(config_dir, library_id)?;
    std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    let mut report = SyncReport::default();
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in &library.folders {
        walk(Path::new(folder), 0, &mut files);
    }
    let present: std::collections::HashSet<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    // Gone, or changed: the old copy goes.
    let mut keep = Vec::new();
    for doc in std::mem::take(&mut library.documents) {
        let Some(source) = doc.source.clone() else {
            keep.push(doc);
            continue;
        };
        let changed = present.contains(&source) && modified_secs(Path::new(&source)) != doc.modified;
        if present.contains(&source) && !changed {
            keep.push(doc);
            continue;
        }
        let _ = std::fs::remove_file(docs.join(format!("{}.txt", doc.id)));
        if let Ok(entries) = std::fs::read_dir(&docs) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&format!("{}.", doc.id)) && name.ends_with(".vec.json") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        if changed {
            report.updated += 1;
        } else {
            report.removed += 1;
        }
    }
    library.documents = keep;
    let known: std::collections::HashSet<String> = library.documents.iter().filter_map(|d| d.source.clone()).collect();
    for path in files {
        let source = path.to_string_lossy().into_owned();
        if known.contains(&source) {
            continue;
        }
        match extract(&path) {
            Ok(text) => {
                let doc = Document {
                    id: uuid::Uuid::new_v4().to_string(),
                    // The path inside the folder, so files with the same name stay apart.
                    name: library
                        .folders
                        .iter()
                        .find_map(|f| path.strip_prefix(f).ok())
                        .map_or_else(|| source.clone(), |p| p.to_string_lossy().into_owned()),
                    chars: text.chars().count(),
                    added_at: Utc::now(),
                    modified: modified_secs(&path),
                    source: Some(source),
                };
                std::fs::write(docs.join(format!("{}.txt", doc.id)), text).map_err(|e| e.to_string())?;
                library.documents.push(doc);
                report.added += 1;
            }
            // Files without text (scans, empty files) are skipped quietly; others say why.
            Err(e) if e.contains("no readable text") || e.contains("no text layer") => {}
            Err(e) => report.errors.push(e),
        }
    }
    // A document re-read after a change was counted as added too.
    report.added = report.added.saturating_sub(report.updated);
    library.synced_at = Some(Utc::now());
    save(config_dir, &library)?;
    Ok(report)
}

/// Mirrors a folder into a library and syncs it.
pub fn add_folder(config_dir: &Path, library_id: &str, folder: &Path) -> Result<SyncReport, String> {
    if !folder.is_dir() {
        return Err("Choose a folder that exists.".into());
    }
    let mut library = load(config_dir, library_id)?;
    let folder = folder.to_string_lossy().into_owned();
    if !library.folders.contains(&folder) {
        library.folders.push(folder);
        save(config_dir, &library)?;
    }
    sync(config_dir, library_id)
}

/// Stops mirroring a folder; its documents leave the library.
pub fn remove_folder(config_dir: &Path, library_id: &str, folder: &str) -> Result<SyncReport, String> {
    let mut library = load(config_dir, library_id)?;
    library.folders.retain(|f| f != folder);
    save(config_dir, &library)?;
    sync(config_dir, library_id)
}

/// Syncs every library that mirrors folders (at start and every few minutes).
pub fn sync_all(config_dir: &Path) {
    for library in list(config_dir).into_iter().filter(|l| !l.folders.is_empty()) {
        match sync(config_dir, &library.id) {
            Ok(r) if r.added + r.updated + r.removed > 0 => log::info!("synced library {}: +{} ~{} -{}", library.name, r.added, r.updated, r.removed),
            Ok(_) => {}
            Err(e) => log::warn!("could not sync library {}: {e}", library.name),
        }
    }
}

/// Overlapping passages, cut at whitespace where possible.
pub fn passages(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + PASSAGE).min(chars.len());
        if end < chars.len() {
            if let Some(space) = chars[start + PASSAGE / 2..end].iter().rposition(|c| c.is_whitespace()) {
                end = start + PASSAGE / 2 + space;
            }
        }
        out.push(chars[start..end].iter().collect::<String>().trim().to_string());
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(OVERLAP).max(start + 1);
    }
    out
}

/// Lowercased words without accents, so "regulação" matches "regulacao".
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(|w| {
            w.to_lowercase()
                .chars()
                .map(|c| match c {
                    'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
                    'é' | 'è' | 'ê' | 'ë' => 'e',
                    'í' | 'ì' | 'î' | 'ï' => 'i',
                    'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
                    'ú' | 'ù' | 'û' | 'ü' => 'u',
                    'ç' => 'c',
                    'ñ' => 'n',
                    other => other,
                })
                .collect()
        })
        .collect()
}

/// BM25 over the passages of the given libraries (all when `only` is empty).
pub fn search(config_dir: &Path, only: &[String], query: &str, k: usize) -> Vec<Passage> {
    let query_terms = tokens(query);
    if query_terms.is_empty() {
        return Vec::new();
    }
    struct Doc {
        library: String,
        document: String,
        passage: usize,
        text: String,
        terms: HashMap<String, f64>,
        len: f64,
    }
    let mut docs = Vec::new();
    for library in list(config_dir).into_iter().filter(|l| only.is_empty() || only.contains(&l.id)) {
        let Ok(dir) = lib_dir(config_dir, &library.id) else { continue };
        for document in &library.documents {
            let Ok(text) = std::fs::read_to_string(dir.join("docs").join(format!("{}.txt", document.id))) else { continue };
            for (i, passage) in passages(&text).into_iter().enumerate() {
                let words = tokens(&passage);
                let mut terms = HashMap::new();
                for w in &words {
                    *terms.entry(w.clone()).or_insert(0.0) += 1.0;
                }
                docs.push(Doc { library: library.name.clone(), document: document.name.clone(), passage: i, text: passage, len: words.len() as f64, terms });
            }
        }
    }
    if docs.is_empty() {
        return Vec::new();
    }
    let n = docs.len() as f64;
    let avg = docs.iter().map(|d| d.len).sum::<f64>() / n;
    let (k1, b) = (1.5, 0.75);
    let idf: HashMap<&str, f64> = query_terms
        .iter()
        .map(|t| {
            let df = docs.iter().filter(|d| d.terms.contains_key(t)).count() as f64;
            (t.as_str(), ((n - df + 0.5) / (df + 0.5) + 1.0).ln())
        })
        .collect();
    let mut scored: Vec<(f64, &Doc)> = docs
        .iter()
        .map(|d| {
            let score = query_terms
                .iter()
                .map(|t| {
                    let tf = d.terms.get(t).copied().unwrap_or(0.0);
                    idf[t.as_str()] * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * d.len / avg.max(1.0)))
                })
                .sum::<f64>();
            (score, d)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(k)
        .map(|(score, d)| Passage { library: d.library.clone(), document: d.document.clone(), passage: d.passage, text: d.text.clone(), score: (score * 100.0).round() / 100.0 })
        .collect()
}

/// The system-prompt line naming the libraries the model can search.
pub fn prompt(libraries: &[Library]) -> Option<String> {
    let useful: Vec<&Library> = libraries.iter().filter(|l| !l.documents.is_empty()).collect();
    if useful.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\nThe user's libraries (search them with `search_library` when a question may be answered \
         by their documents, read a whole document with `read_document` when passages are not enough, and cite \
         the document name like [Doc name]):",
    );
    for l in useful {
        out.push_str(&format!("\n- {} ({} documents){}", l.name, l.documents.len(), if l.description.is_empty() { String::new() } else { format!(": {}", l.description) }));
    }
    Some(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn whole_documents_are_read_in_parts() {
        let config = tempfile::tempdir().unwrap();
        let lib = create(config.path(), "Contracts", "").unwrap();
        add_text(config.path(), &lib.id, "lease.md", &"clause ".repeat(12_000)).unwrap();
        let first = read_document(config.path(), "", "LEASE", 1).unwrap();
        assert!(first.starts_with("# lease.md (Contracts), part 1 of 3"), "{}", &first[..60]);
        assert!(read_document(config.path(), "Contracts", "lease", 3).is_ok());
        assert!(read_document(config.path(), "", "lease.md", 4).is_err());
        assert!(read_document(config.path(), "", "missing", 1).is_err());
    }


    #[test]
    fn folders_are_mirrored() {
        let config = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("notes/.git")).unwrap();
        std::fs::write(folder.path().join("a.md"), "Alpha project kickoff notes").unwrap();
        std::fs::write(folder.path().join("notes/b.txt"), "Beta budget is 40k").unwrap();
        std::fs::write(folder.path().join("notes/.git/config"), "hidden").unwrap();
        std::fs::write(folder.path().join("photo.png"), [0u8, 1, 2]).unwrap();
        let lib = create(config.path(), "Work", "").unwrap();
        let r = add_folder(config.path(), &lib.id, folder.path()).unwrap();
        assert_eq!((r.added, r.updated, r.removed), (2, 0, 0));
        let names: Vec<String> = load(config.path(), &lib.id).unwrap().documents.iter().map(|d| d.name.clone()).collect();
        assert!(names.contains(&"a.md".to_string()) && names.iter().any(|n| n.ends_with("b.txt")), "{names:?}");
        assert_eq!(search(config.path(), &[lib.id.clone()], "budget", 3)[0].document, names.iter().find(|n| n.ends_with("b.txt")).unwrap().clone());
        // Nothing changed: nothing to do.
        assert_eq!(sync(config.path(), &lib.id).unwrap(), SyncReport::default());
        // A change, a deletion and a new file.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(folder.path().join("a.md"), "Alpha project moved to March").unwrap();
        std::fs::remove_file(folder.path().join("notes/b.txt")).unwrap();
        std::fs::write(folder.path().join("c.csv"), "item,cost\nchairs,300").unwrap();
        let r = sync(config.path(), &lib.id).unwrap();
        assert_eq!((r.added, r.updated, r.removed), (1, 1, 1));
        assert!(!search(config.path(), &[lib.id.clone()], "March", 3).is_empty());
        // Unmirrored: its documents leave.
        let r = remove_folder(config.path(), &lib.id, &folder.path().to_string_lossy()).unwrap();
        assert_eq!(r.removed, 2);
        assert!(load(config.path(), &lib.id).unwrap().documents.is_empty());
    }

    use super::*;

    fn zipped(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, body) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn zip_archives_are_read_file_by_file() {
        let archive = zipped(&[("notes/a.md", "# Alpha\nfirst"), ("b.txt", "second"), ("img.png", "\u{0}\u{1}binary"), ("__MACOSX/x", "junk")]);
        let text = zip_text(&archive, 10_000).unwrap();
        assert!(text.contains("--- b.txt ---\nsecond") && text.contains("--- notes/a.md ---\n# Alpha"), "{text}");
        assert!(text.ends_with("(Not read: img.png)"), "{text}");
        assert!(zip_text(b"nope", 100).is_err());
    }

    #[test]
    fn office_documents_are_read_as_text() {
        let docx = zipped(&[(
            "word/document.xml",
            r#"<w:document><w:body><w:p><w:r><w:t>Quarterly</w:t></w:r><w:r><w:t xml:space="preserve"> report &amp; plan</w:t></w:r></w:p>
            <w:p><w:r><w:t>Revenue</w:t><w:tab/><w:t>12 &#8364;</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        assert_eq!(office_text(&docx, "docx").unwrap(), "Quarterly report & plan\nRevenue\t12 €");
        let pptx = zipped(&[
            ("ppt/slides/slide10.xml", "<p:sld><a:p><a:r><a:t>Last</a:t></a:r></a:p></p:sld>"),
            ("ppt/slides/slide2.xml", "<p:sld><a:p><a:r><a:t>Second</a:t></a:r></a:p></p:sld>"),
            ("ppt/slides/slide1.xml", "<p:sld><a:p><a:r><a:t>Title</a:t></a:r></a:p></p:sld>"),
        ]);
        assert_eq!(office_text(&pptx, "pptx").unwrap(), "Slide 1:\nTitle\n\nSlide 2:\nSecond\n\nSlide 3:\nLast");
        let odt = zipped(&[("content.xml", "<office:text><text:h>Heading</text:h><text:p>One<text:s/>two</text:p></office:text>")]);
        assert_eq!(office_text(&odt, "odt").unwrap(), "Heading\nOne two");
        assert!(office_text(b"not a zip", "docx").is_err());
        // Libraries index them too.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.docx");
        std::fs::write(&path, &docx).unwrap();
        assert_eq!(extract(&path).unwrap(), "Quarterly report & plan Revenue 12 €");
    }

    #[test]
    fn passages_overlap_and_cover_the_text() {
        let text = "word ".repeat(1000);
        let parts = passages(&text);
        assert!(parts.len() >= 4);
        assert!(parts.iter().all(|p| p.chars().count() <= PASSAGE));
        assert_eq!(passages("short").len(), 1);
    }

    #[test]
    fn tokens_ignore_case_and_accents() {
        assert_eq!(tokens("Regulação da ERSE, 2026!"), ["regulacao", "da", "erse", "2026"]);
    }

    #[test]
    fn libraries_index_and_rank_documents() {
        let dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let lib = create(dir.path(), "Energy", "Regulation notes").unwrap();
        let a = files.path().join("tariffs.md");
        std::fs::write(&a, "ERSE sets regulated electricity tariffs every year after public consultation.").unwrap();
        let b = files.path().join("garden.txt");
        std::fs::write(&b, "Tomatoes need sun and regular watering in summer.").unwrap();
        let html = files.path().join("page.html");
        std::fs::write(&html, "<html><head><title>x</title></head><body><p>Gas market rules</p></body></html>").unwrap();
        add(dir.path(), &lib.id, &a).unwrap();
        add(dir.path(), &lib.id, &b).unwrap();
        let lib = add(dir.path(), &lib.id, &html).unwrap();
        assert_eq!(lib.documents.len(), 3);

        let hits = search(dir.path(), &[], "electricity tariffs consultation", 5);
        assert_eq!(hits[0].document, "tariffs.md");
        assert!(search(dir.path(), &[], "gas", 5).iter().any(|h| h.document == "page.html"));
        assert!(search(dir.path(), &["other".into()], "tariffs", 5).is_empty());

        let binary = files.path().join("x.bin");
        std::fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert!(add(dir.path(), &lib.id, &binary).is_err());

        let doc = lib.documents[1].id.clone();
        assert_eq!(remove(dir.path(), &lib.id, &doc).unwrap().documents.len(), 2);
        assert!(prompt(&list(dir.path())).unwrap().contains("Energy (2 documents)"));
        delete(dir.path(), &lib.id).unwrap();
        assert!(list(dir.path()).is_empty());
    }
}
