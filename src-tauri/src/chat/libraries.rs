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

/// The text of a file, by its kind.
pub fn extract(path: &Path) -> Result<String, String> {
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
                return Err(format!("{} is not a text document (PDF, HTML, Markdown, text, code and data files are supported).", path.display()));
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
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
         by their documents, and cite the document name like [Doc name]):",
    );
    for l in useful {
        out.push_str(&format!("\n- {} ({} documents){}", l.name, l.documents.len(), if l.description.is_empty() { String::new() } else { format!(": {}", l.description) }));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
