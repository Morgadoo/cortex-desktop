//! Mirrored folders: walking them and keeping the library in step with what changed.

use super::*;

/// File types a mirrored folder indexes.
pub(super) const SYNC_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "pptx", "odt", "odp", "md", "markdown", "txt", "html", "htm", "csv", "json",
    "yaml", "yml", "xml", "rs", "py", "js", "ts", "go", "java", "c", "cpp", "h", "sql", "log",
    "rst", "org", "tex",
];
/// Files indexed per folder, and how deep a folder is walked.
pub(super) const SYNC_MAX_FILES: usize = 2_000;
pub(super) const SYNC_MAX_DEPTH: usize = 6;

/// What a sync changed.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub errors: Vec<String>,
}

pub(super) fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > SYNC_MAX_DEPTH || out.len() >= SYNC_MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hidden files and folders, and build and dependency folders, are not documents.
        if name.starts_with('.')
            || matches!(
                name.as_str(),
                "node_modules" | "target" | "__pycache__" | "venv" | ".venv"
            )
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, depth + 1, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SYNC_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
            if out.len() >= SYNC_MAX_FILES {
                return;
            }
        }
    }
}

pub(super) fn modified_secs(path: &Path) -> Option<i64> {
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
    let present: std::collections::HashSet<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // Gone, or changed: the old copy goes.
    let mut keep = Vec::new();
    for doc in std::mem::take(&mut library.documents) {
        // Documents from connectors are kept in step by `sync_connectors`.
        let Some(source) = doc
            .source
            .clone()
            .filter(|s| !s.starts_with(CONNECTOR_SOURCE))
        else {
            keep.push(doc);
            continue;
        };
        let changed =
            present.contains(&source) && modified_secs(Path::new(&source)) != doc.modified;
        if present.contains(&source) && !changed {
            keep.push(doc);
            continue;
        }
        drop_document_files(&docs, &doc.id);
        if changed {
            report.updated += 1;
        } else {
            report.removed += 1;
        }
    }
    library.documents = keep;
    let known: std::collections::HashSet<String> = library
        .documents
        .iter()
        .filter_map(|d| d.source.clone())
        .collect();
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
                crate::chat::at_rest::write(docs.join(format!("{}.txt", doc.id)), text)
                    .map_err(|e| e.to_string())?;
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

/// Removes a document's text and its embeddings.
pub(super) fn drop_document_files(docs: &Path, id: &str) {
    let _ = std::fs::remove_file(docs.join(format!("{id}.txt")));
    if let Ok(entries) = std::fs::read_dir(docs) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{id}.")) && name.ends_with(".vec.json") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
