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

use crate::chat::store::{chat_dir, valid_id, write_json};

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
    /// Connectors whose documents (MCP resources) the library mirrors, by connector id.
    #[serde(default)]
    pub connectors: Vec<String>,
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
    let Ok(entries) = std::fs::read_dir(root(config_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<Library> = entries
        .filter_map(Result::ok)
        .filter_map(|e| crate::chat::at_rest::read_to_string(e.path().join("library.json")).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

pub fn load(config_dir: &Path, id: &str) -> Result<Library, String> {
    let text = crate::chat::at_rest::read_to_string(lib_dir(config_dir, id)?.join("library.json"))
        .map_err(|_| "No such library.".to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn save(config_dir: &Path, library: &Library) -> Result<(), String> {
    write_json(
        &lib_dir(config_dir, &library.id)?.join("library.json"),
        library,
    )
    .map_err(String::from)
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
        connectors: Vec::new(),
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

/// Adds a file's text to a library.
pub fn add(config_dir: &Path, library_id: &str, path: &Path) -> Result<Library, String> {
    let text = extract(path)?;
    let name = path
        .file_name()
        .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
    add_text(config_dir, library_id, &name, &text)
}

/// Image files, whose text is read by a model that can see (`ocr`).
pub fn is_image(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Adds text already extracted (from a file, or read from an image) as a document.
pub fn add_text(
    config_dir: &Path,
    library_id: &str,
    name: &str,
    text: &str,
) -> Result<Library, String> {
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
    crate::chat::at_rest::write(docs.join(format!("{}.txt", doc.id)), text)
        .map_err(|e| e.to_string())?;
    library.documents.push(doc);
    save(config_dir, &library)?;
    Ok(library)
}

/// A date cutoff from what a model or a person wrote: `2026-03-01`, or `30d`, `6m`, `1y` ago.
pub fn parse_since(text: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(&t, "%Y-%m-%d") {
        return date.and_hms_opt(0, 0, 0).map(|d| d.and_utc());
    }
    let (n, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok()?;
    let days = match unit.trim() {
        "d" | "day" | "days" => n,
        "w" | "week" | "weeks" => n * 7,
        "m" | "month" | "months" => n * 30,
        "y" | "year" | "years" => n * 365,
        _ => return None,
    };
    Some(now - chrono::Duration::days(days))
}

/// Keeps the passages from documents changed on or after `since` (Onyx's time cutoff).
pub fn changed_since(
    config_dir: &Path,
    passages: Vec<Passage>,
    since: DateTime<Utc>,
) -> Vec<Passage> {
    let dates: HashMap<(String, String), DateTime<Utc>> = list(config_dir)
        .into_iter()
        .flat_map(|l| {
            l.documents
                .into_iter()
                .map(move |d| ((l.name.clone(), d.name.clone()), d.changed_at()))
        })
        .collect();
    passages
        .into_iter()
        .filter(|p| {
            dates
                .get(&(p.library.clone(), p.document.clone()))
                .is_some_and(|d| *d >= since)
        })
        .collect()
}

/// Where a library keeps its documents' text (and their cached vectors).
pub(in crate::chat::grounding) fn docs_dir(
    config_dir: &Path,
    library_id: &str,
) -> Result<PathBuf, String> {
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
pub fn read_document(
    config_dir: &Path,
    library: &str,
    name: &str,
    part: usize,
) -> Result<String, String> {
    let wanted = name.trim().to_lowercase();
    let found = list(config_dir)
        .into_iter()
        .filter(|l| library.trim().is_empty() || l.name.eq_ignore_ascii_case(library.trim()))
        .find_map(|l| {
            let doc = l
                .documents
                .iter()
                .find(|d| d.name.to_lowercase() == wanted)
                .or_else(|| {
                    l.documents
                        .iter()
                        .find(|d| d.name.to_lowercase().contains(&wanted))
                })
                .cloned()?;
            Some((l, doc))
        });
    let Some((library, doc)) = found else {
        return Err(format!(
            "No library document is named {name}; use a name from search_library."
        ));
    };
    let text = crate::chat::at_rest::read_to_string(
        docs_dir(config_dir, &library.id)?.join(format!("{}.txt", doc.id)),
    )
    .map_err(|e| e.to_string())?;
    let chars: Vec<char> = text.chars().collect();
    let parts = chars.len().div_ceil(READ_PART).max(1);
    if part > parts {
        return Err(format!("{} has {parts} part(s).", doc.name));
    }
    let slice: String = chars[(part - 1) * READ_PART..(part * READ_PART).min(chars.len())]
        .iter()
        .collect();
    Ok(format!(
        "# {} ({}), part {part} of {parts}\n\n{slice}",
        doc.name, library.name
    ))
}

/// Saves a web page's readable text into a library (Onyx's web connector, Open WebUI's "add a
/// webpage"); the document is named after the page and starts with its address.
pub async fn add_web_page(
    config_dir: &Path,
    library_id: &str,
    url: &str,
) -> Result<Library, String> {
    let parsed =
        url::Url::parse(url.trim()).map_err(|_| "That is not a web address.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https pages can be saved.".into());
    }
    let client = crate::support::http::browser_client(std::time::Duration::from_secs(25))?;
    let response = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| format!("The page could not be fetched: {e}"))?;
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
    let host = parsed
        .host_str()
        .unwrap_or("page")
        .trim_start_matches("www.")
        .to_string();
    let name: String = if title.trim().is_empty() {
        format!("{host}{}", parsed.path())
    } else {
        format!("{} ({host})", title.trim())
    }
    .chars()
    .take(120)
    .collect();
    add_text(
        config_dir,
        library_id,
        &name,
        &format!("Source: {parsed}\n\n{text}"),
    )
}

/// The indexed text of a document, for its preview (at most `max` characters).
pub fn document_text(
    config_dir: &Path,
    library_id: &str,
    document_id: &str,
    max: usize,
) -> Result<String, String> {
    let library = load(config_dir, library_id)?;
    let doc = library
        .documents
        .iter()
        .find(|d| d.id == document_id)
        .ok_or("That document is gone.")?;
    let text = crate::chat::at_rest::read_to_string(
        docs_dir(config_dir, &library.id)?.join(format!("{}.txt", doc.id)),
    )
    .map_err(|e| e.to_string())?;
    Ok(if text.chars().count() > max {
        format!(
            "{}\n\n… (the rest is cut here; the assistant reads it all with read_document)",
            text.chars().take(max).collect::<String>()
        )
    } else {
        text
    })
}

/// Mirrors a folder into a library and syncs it.
pub fn add_folder(
    config_dir: &Path,
    library_id: &str,
    folder: &Path,
) -> Result<SyncReport, String> {
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
pub fn remove_folder(
    config_dir: &Path,
    library_id: &str,
    folder: &str,
) -> Result<SyncReport, String> {
    let mut library = load(config_dir, library_id)?;
    library.folders.retain(|f| f != folder);
    save(config_dir, &library)?;
    sync(config_dir, library_id)
}

/// Syncs every library that mirrors folders (at start and every few minutes).
pub fn sync_all(config_dir: &Path) {
    for library in list(config_dir)
        .into_iter()
        .filter(|l| !l.folders.is_empty())
    {
        match sync(config_dir, &library.id) {
            Ok(r) if r.added + r.updated + r.removed > 0 => log::info!(
                "synced library {}: +{} ~{} -{}",
                library.name,
                r.added,
                r.updated,
                r.removed
            ),
            Ok(_) => {}
            Err(e) => log::warn!("could not sync library {}: {e}", library.name),
        }
    }
}

mod archives;
mod connectors;
mod folders;
mod reading;
mod search;
pub use archives::*;
pub use connectors::*;
pub use folders::*;
pub use reading::*;
pub use search::*;

#[cfg(test)]
mod connector_tests {
    use super::*;
    use crate::tools::auth::{AuthMechanism, Readiness};
    use crate::tools::error::ToolError;
    use crate::tools::registry::{Connector, ConnectorDescriptor, ToolDefinition};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    /// A connector offering documents, which the test changes between syncs.
    struct Wiki {
        pages: Mutex<Vec<(String, String)>>,
        down: Mutex<bool>,
    }

    #[async_trait::async_trait]
    impl Connector for Wiki {
        fn id(&self) -> &str {
            "wiki"
        }
        fn descriptor(&self) -> ConnectorDescriptor {
            ConnectorDescriptor {
                id: "wiki".into(),
                display_name: "Team wiki".into(),
                description: None,
                version: "1".into(),
                icon: None,
                auth: AuthMechanism::None,
            }
        }
        fn tools(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }
        fn readiness(&self) -> Readiness {
            Readiness::Ready
        }
        async fn invoke(&self, _: &str, _: Value) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
        async fn resources(&self) -> Result<Vec<Value>, ToolError> {
            if *self.down.lock().unwrap() {
                return Err(ToolError::Unreachable("offline".into()));
            }
            Ok(self
                .pages
                .lock()
                .unwrap()
                .iter()
                .map(|(uri, _)| json!({"uri": uri, "name": uri.rsplit('/').next()}))
                .collect())
        }
        async fn read_resource(&self, uri: &str) -> Result<String, ToolError> {
            self.pages
                .lock()
                .unwrap()
                .iter()
                .find(|(u, _)| u == uri)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| ToolError::ExecutionFailed("gone".into()))
        }
    }

    #[tokio::test]
    async fn connector_documents_are_mirrored_and_kept_in_step() {
        let dir = tempfile::tempdir().unwrap();
        let lib = create(dir.path(), "Wiki", "").unwrap();
        let wiki = Arc::new(Wiki {
            pages: Mutex::new(vec![
                (
                    "wiki://onboarding".into(),
                    "Laptops are ordered by Rita.".into(),
                ),
                ("wiki://holidays".into(), "We close on 24 December.".into()),
            ]),
            down: Mutex::new(false),
        });
        let connectors: Vec<Arc<dyn Connector>> = vec![wiki.clone()];
        set_connector(dir.path(), &lib.id, "wiki", true).unwrap();
        let r = sync_connectors(dir.path(), &lib.id, &connectors)
            .await
            .unwrap();
        assert_eq!((r.added, r.updated, r.removed), (2, 0, 0));
        assert!(search(
            dir.path(),
            std::slice::from_ref(&lib.id),
            "who orders laptops",
            3
        )
        .iter()
        .any(|p| p.text.contains("Rita")));
        // Nothing changed: nothing to do; a folder sync leaves connector documents alone.
        let r = sync_connectors(dir.path(), &lib.id, &connectors)
            .await
            .unwrap();
        assert_eq!((r.added, r.updated, r.removed), (0, 0, 0));
        sync(dir.path(), &lib.id).unwrap();
        assert_eq!(load(dir.path(), &lib.id).unwrap().documents.len(), 2);
        // A page changes and one is deleted.
        *wiki.pages.lock().unwrap() = vec![(
            "wiki://onboarding".into(),
            "Laptops are ordered by Tomas.".into(),
        )];
        let r = sync_connectors(dir.path(), &lib.id, &connectors)
            .await
            .unwrap();
        assert_eq!((r.added, r.updated, r.removed), (0, 1, 1));
        assert!(search(
            dir.path(),
            std::slice::from_ref(&lib.id),
            "laptops ordered",
            3
        )
        .iter()
        .any(|p| p.text.contains("Tomas")));
        // Offline: its documents stay.
        *wiki.down.lock().unwrap() = true;
        let r = sync_connectors(dir.path(), &lib.id, &connectors)
            .await
            .unwrap();
        assert_eq!(r.removed, 0);
        assert!(!r.errors.is_empty());
        assert_eq!(load(dir.path(), &lib.id).unwrap().documents.len(), 1);
        // No longer mirrored: its documents leave.
        set_connector(dir.path(), &lib.id, "wiki", false).unwrap();
        let r = sync_connectors(dir.path(), &lib.id, &connectors)
            .await
            .unwrap();
        assert_eq!(r.removed, 1);
        assert!(load(dir.path(), &lib.id).unwrap().documents.is_empty());
    }

    #[test]
    fn resource_text_joins_text_parts() {
        let v = json!({"contents": [{"uri": "a", "text": "one"}, {"uri": "b", "blob": "AA=="}, {"uri": "c", "text": "two"}]});
        assert_eq!(
            crate::tools::connectors::mcp::resource_text(&v),
            "one\n\ntwo"
        );
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn whole_documents_are_read_in_parts() {
        let config = tempfile::tempdir().unwrap();
        let lib = create(config.path(), "Contracts", "").unwrap();
        add_text(
            config.path(),
            &lib.id,
            "lease.md",
            &"clause ".repeat(12_000),
        )
        .unwrap();
        let first = read_document(config.path(), "", "LEASE", 1).unwrap();
        assert!(
            first.starts_with("# lease.md (Contracts), part 1 of 3"),
            "{}",
            &first[..60]
        );
        assert!(read_document(config.path(), "Contracts", "lease", 3).is_ok());
        assert!(read_document(config.path(), "", "lease.md", 4).is_err());
        assert!(read_document(config.path(), "", "missing", 1).is_err());
    }

    #[test]
    fn folders_are_mirrored() {
        let config = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("notes/.git")).unwrap();
        crate::chat::at_rest::write(folder.path().join("a.md"), "Alpha project kickoff notes")
            .unwrap();
        crate::chat::at_rest::write(folder.path().join("notes/b.txt"), "Beta budget is 40k")
            .unwrap();
        crate::chat::at_rest::write(folder.path().join("notes/.git/config"), "hidden").unwrap();
        crate::chat::at_rest::write(folder.path().join("photo.png"), [0u8, 1, 2]).unwrap();
        let lib = create(config.path(), "Work", "").unwrap();
        let r = add_folder(config.path(), &lib.id, folder.path()).unwrap();
        assert_eq!((r.added, r.updated, r.removed), (2, 0, 0));
        let names: Vec<String> = load(config.path(), &lib.id)
            .unwrap()
            .documents
            .iter()
            .map(|d| d.name.clone())
            .collect();
        assert!(
            names.contains(&"a.md".to_string()) && names.iter().any(|n| n.ends_with("b.txt")),
            "{names:?}"
        );
        assert_eq!(
            search(config.path(), std::slice::from_ref(&lib.id), "budget", 3)[0].document,
            names.iter().find(|n| n.ends_with("b.txt")).unwrap().clone()
        );
        // Nothing changed: nothing to do.
        assert_eq!(sync(config.path(), &lib.id).unwrap(), SyncReport::default());
        // A change, a deletion and a new file.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        crate::chat::at_rest::write(folder.path().join("a.md"), "Alpha project moved to March")
            .unwrap();
        std::fs::remove_file(folder.path().join("notes/b.txt")).unwrap();
        crate::chat::at_rest::write(folder.path().join("c.csv"), "item,cost\nchairs,300").unwrap();
        let r = sync(config.path(), &lib.id).unwrap();
        assert_eq!((r.added, r.updated, r.removed), (1, 1, 1));
        assert!(!search(config.path(), std::slice::from_ref(&lib.id), "March", 3).is_empty());
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
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn zip_archives_are_read_file_by_file() {
        let archive = zipped(&[
            ("notes/a.md", "# Alpha\nfirst"),
            ("b.txt", "second"),
            ("img.png", "\u{0}\u{1}binary"),
            ("__MACOSX/x", "junk"),
        ]);
        let text = zip_text(&archive, 10_000).unwrap();
        assert!(
            text.contains("--- b.txt ---\nsecond") && text.contains("--- notes/a.md ---\n# Alpha"),
            "{text}"
        );
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
        assert_eq!(
            office_text(&docx, "docx").unwrap(),
            "Quarterly report & plan\nRevenue\t12 €"
        );
        let pptx = zipped(&[
            (
                "ppt/slides/slide10.xml",
                "<p:sld><a:p><a:r><a:t>Last</a:t></a:r></a:p></p:sld>",
            ),
            (
                "ppt/slides/slide2.xml",
                "<p:sld><a:p><a:r><a:t>Second</a:t></a:r></a:p></p:sld>",
            ),
            (
                "ppt/slides/slide1.xml",
                "<p:sld><a:p><a:r><a:t>Title</a:t></a:r></a:p></p:sld>",
            ),
        ]);
        assert_eq!(
            office_text(&pptx, "pptx").unwrap(),
            "Slide 1:\nTitle\n\nSlide 2:\nSecond\n\nSlide 3:\nLast"
        );
        let odt = zipped(&[(
            "content.xml",
            "<office:text><text:h>Heading</text:h><text:p>One<text:s/>two</text:p></office:text>",
        )]);
        assert_eq!(office_text(&odt, "odt").unwrap(), "Heading\nOne two");
        assert!(office_text(b"not a zip", "docx").is_err());
        // Libraries index them too.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.docx");
        crate::chat::at_rest::write(&path, &docx).unwrap();
        assert_eq!(
            extract(&path).unwrap(),
            "Quarterly report & plan Revenue 12 €"
        );
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
    fn libraries_index_and_rank_documents() {
        let dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let lib = create(dir.path(), "Energy", "Regulation notes").unwrap();
        let a = files.path().join("tariffs.md");
        crate::chat::at_rest::write(
            &a,
            "ERSE sets regulated electricity tariffs every year after public consultation.",
        )
        .unwrap();
        let b = files.path().join("garden.txt");
        crate::chat::at_rest::write(&b, "Tomatoes need sun and regular watering in summer.")
            .unwrap();
        let html = files.path().join("page.html");
        crate::chat::at_rest::write(
            &html,
            "<html><head><title>x</title></head><body><p>Gas market rules</p></body></html>",
        )
        .unwrap();
        add(dir.path(), &lib.id, &a).unwrap();
        add(dir.path(), &lib.id, &b).unwrap();
        let lib = add(dir.path(), &lib.id, &html).unwrap();
        assert_eq!(lib.documents.len(), 3);

        let hits = search(dir.path(), &[], "electricity tariffs consultation", 5);
        assert_eq!(hits[0].document, "tariffs.md");
        assert!(search(dir.path(), &[], "gas", 5)
            .iter()
            .any(|h| h.document == "page.html"));
        assert!(search(dir.path(), &["other".into()], "tariffs", 5).is_empty());

        let binary = files.path().join("x.bin");
        crate::chat::at_rest::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert!(add(dir.path(), &lib.id, &binary).is_err());

        let doc = lib.documents[1].id.clone();
        assert_eq!(
            remove(dir.path(), &lib.id, &doc).unwrap().documents.len(),
            2
        );
        assert!(prompt(&list(dir.path()))
            .unwrap()
            .contains("Energy (2 documents)"));
        delete(dir.path(), &lib.id).unwrap();
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn notion_exports_import_as_named_documents() {
        assert_eq!(
            archive_name("Trip plan 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d.md"),
            "Trip plan.md"
        );
        assert_eq!(archive_name("Projects 0123456789abcdef0123456789abcdef/Budget 0123456789abcdef0123456789abcdee.csv"), "Projects / Budget.csv");
        assert_eq!(archive_name("notes/plain name.md"), "notes / plain name.md");
        let zip_of = |entries: &[(&str, &[u8])]| {
            let mut bytes = Vec::new();
            {
                let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
                for (name, data) in entries {
                    w.start_file(*name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    std::io::Write::write_all(&mut w, data).unwrap();
                }
                w.finish().unwrap();
            }
            bytes
        };
        // Notion splits big exports into parts: a ZIP of ZIPs.
        let part = zip_of(&[
            (
                "Home 0123456789abcdef0123456789abcdef.md",
                b"# Home\nWelcome to the team wiki.",
            ),
            ("Home/logo.png", b"\x89PNG"),
        ]);
        let export = zip_of(&[
            ("Export-Part-1.zip", &part),
            (
                "Tasks 0123456789abcdef0123456789abcdef.csv",
                b"Name,Status\nShip,Done",
            ),
            ("__MACOSX/x.md", b"junk"),
        ]);
        let files = archive_files(&export, 0).unwrap();
        let names: Vec<String> = files.iter().map(|(p, _)| archive_name(p)).collect();
        assert_eq!(names, ["Home.md", "Tasks.csv"]);

        let dir = std::env::temp_dir().join(format!("cortex-archive-{}", std::process::id()));
        let library = create(&dir, "Wiki", "").unwrap();
        let report = import_archive(&dir, &library.id, &export).unwrap();
        assert_eq!(report.added, 2);
        assert!(report.skipped.is_empty());
        assert_eq!(
            report
                .library
                .documents
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            ["Home.md", "Tasks.csv"]
        );
        assert!(search(&dir, &[], "team wiki", 3)
            .iter()
            .any(|p| p.text.contains("Welcome")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_time_cutoff_keeps_recent_documents() {
        let now = Utc::now();
        assert_eq!(
            parse_since("2026-03-01", now)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "2026-03-01"
        );
        assert_eq!((now - parse_since("30d", now).unwrap()).num_days(), 30);
        assert_eq!((now - parse_since("2 weeks", now).unwrap()).num_days(), 14);
        assert_eq!((now - parse_since("1y", now).unwrap()).num_days(), 365);
        assert!(parse_since("lately", now).is_none());
        assert!(parse_since("", now).is_none());

        let dir = std::env::temp_dir().join(format!("cortex-since-{}", std::process::id()));
        let lib = create(&dir, "Notes", "").unwrap();
        add_text(&dir, &lib.id, "old.md", "budget figures for the old plan").unwrap();
        add_text(&dir, &lib.id, "new.md", "budget figures for the new plan").unwrap();
        // The first document is from two years ago.
        let mut l = load(&dir, &lib.id).unwrap();
        l.documents[0].added_at = now - chrono::Duration::days(730);
        save(&dir, &l).unwrap();
        let hits = search(&dir, &[], "budget figures", 5);
        assert_eq!(hits.len(), 2);
        let recent = changed_since(&dir, hits, parse_since("1y", now).unwrap());
        assert_eq!(
            recent
                .iter()
                .map(|p| p.document.as_str())
                .collect::<Vec<_>>(),
            ["new.md"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
