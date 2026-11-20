//! Importing an archive of documents into a library.

use super::*;

/// Extensions an archive import reads; the rest (images, styles, scripts) are passed over.
pub(super) const ARCHIVE_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "csv", "tsv", "html", "htm", "pdf", "docx", "pptx", "odt", "odp",
    "json", "yaml", "yml", "org", "rst",
];

/// A page's name without the id Notion adds (`Trip plan 1a2b…(32 hex)`), with its folders:
/// `Projects/Trip plan 1a2b….md` → `Projects / Trip plan.md`.
pub fn archive_name(path: &str) -> String {
    let clean = |part: &str| -> String {
        let (stem, ext) = match part.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && ext.len() <= 8 => (stem, Some(ext)),
            _ => (part, None),
        };
        let stem = match stem.rsplit_once(' ') {
            Some((head, id)) if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) => head,
            _ => stem,
        };
        let stem = stem.trim();
        match ext {
            Some(ext) => format!("{stem}.{ext}"),
            None => stem.to_string(),
        }
    };
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(clean)
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The readable files of a ZIP archive (a Notion or Confluence export, a folder of notes), as
/// names and bytes. ZIPs inside it (Notion splits large exports into parts) are opened too.
pub fn archive_files(bytes: &[u8], depth: u8) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::io::Read as _;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "That is not a ZIP archive.".to_string())?;
    let mut out = Vec::new();
    let mut total = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file() || entry.size() > 50 * 1024 * 1024 {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let path = path.to_string_lossy().replace('\\', "/");
        if path
            .split('/')
            .any(|p| p.starts_with("__MACOSX") || p.starts_with('.'))
        {
            continue;
        }
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "zip" && !ARCHIVE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
        total += data.len();
        if total > 300 * 1024 * 1024 {
            return Err("That archive is too large to import.".into());
        }
        if ext == "zip" {
            if depth < 2 {
                out.extend(archive_files(&data, depth + 1)?);
            }
        } else {
            out.push((path, data));
        }
    }
    Ok(out)
}

/// What an archive import did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveReport {
    pub library: Library,
    pub added: usize,
    pub skipped: Vec<String>,
}

/// Adds every readable file of a ZIP archive to a library, one document each.
pub fn import_archive(
    config_dir: &Path,
    library_id: &str,
    bytes: &[u8],
) -> Result<ArchiveReport, String> {
    let files = archive_files(bytes, 0)?;
    if files.is_empty() {
        return Err("No pages or documents were found in that archive.".into());
    }
    let scratch = std::env::temp_dir().join(format!("cortex-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let mut added = 0;
    let mut skipped = Vec::new();
    let mut library = load(config_dir, library_id)?;
    for (i, (path, data)) in files.iter().enumerate() {
        let name = archive_name(path);
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        // Read through a file, so every kind is read as when it is added from disk.
        let file = scratch.join(format!("{i}.{ext}"));
        let result = crate::chat::at_rest::write(&file, data)
            .map_err(|e| e.to_string())
            .and_then(|()| read_text(&file));
        let _ = std::fs::remove_file(&file);
        match result.and_then(|text| add_text(config_dir, library_id, &name, &text)) {
            Ok(updated) => {
                library = updated;
                added += 1;
            }
            Err(e) => skipped.push(format!("{name}: {e}")),
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(ArchiveReport {
        library,
        added,
        skipped,
    })
}

impl Document {
    /// When the document last changed: its file's time for a mirrored folder, else when it was added.
    pub fn changed_at(&self) -> DateTime<Utc> {
        self.modified
            .and_then(|secs| DateTime::from_timestamp(secs, 0))
            .unwrap_or(self.added_at)
    }
}
