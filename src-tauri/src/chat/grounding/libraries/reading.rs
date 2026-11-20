//! Reading a document's text: Office files, ZIP archives, plain text and the extraction engines.

use super::*;
use crate::chat::error::ChatError;

/// Office documents (Word, PowerPoint, OpenDocument text and slides) by their zipped XML.
pub const OFFICE_EXTENSIONS: &[&str] = &["docx", "pptx", "odt", "odp"];

/// The text of an Office document: paragraphs on their own lines, slides in order.
pub fn office_text(bytes: &[u8], ext: &str) -> Result<String, ChatError> {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "The document could not be opened (not an Office file?).".to_string())
        .map_err(ChatError::Invalid)?;
    let mut parts: Vec<String> = match ext {
        "docx" => {
            let mut names = vec!["word/document.xml".to_string()];
            names.extend(
                zip.file_names()
                    .filter(|n| n.starts_with("word/footnotes") && n.ends_with(".xml"))
                    .map(str::to_string),
            );
            names
        }
        "pptx" => {
            let mut slides: Vec<(u32, String)> = zip
                .file_names()
                .filter_map(|n| {
                    n.strip_prefix("ppt/slides/slide")
                        .and_then(|r| r.strip_suffix(".xml"))
                        .and_then(|d| d.parse().ok())
                        .map(|d| (d, n.to_string()))
                })
                .collect();
            slides.sort();
            slides.into_iter().map(|(_, n)| n).collect()
        }
        _ => vec!["content.xml".to_string()],
    };
    parts.retain(|n| zip.by_name(n).is_ok());
    if parts.is_empty() {
        return Err(ChatError::Invalid("The document has no text part.".into()));
    }
    let mut out = Vec::new();
    for (i, name) in parts.iter().enumerate() {
        let mut xml = String::new();
        zip.by_name(name)
            .map_err(ChatError::internal)?
            .read_to_string(&mut xml)
            .map_err(ChatError::internal)?;
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
pub(super) fn xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let between = &rest[..start];
        // Indentation between tags in pretty-printed XML is not text.
        if !(between.trim().is_empty() && between.contains('\n')) {
            out.push_str(&decode_entities(between));
        }
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start + 1..start + end];
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
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

pub(super) fn decode_entities(s: &str) -> String {
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
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
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
pub fn zip_text(bytes: &[u8], budget: usize) -> Result<String, ChatError> {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "That is not a ZIP archive.".to_string())
        .map_err(ChatError::Invalid)?;
    let mut out = String::new();
    let mut skipped = Vec::new();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|n| {
            !n.ends_with('/')
                && !n.starts_with("__MACOSX/")
                && !n.split('/').any(|p| p.starts_with('.'))
        })
        .collect();
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
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
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
                let t: String = crate::support::text::clip(&t, room);
                out.push_str(&format!("--- {name} ---\n{}\n\n", t.trim_end()));
            }
            _ => skipped.push(name.clone()),
        }
    }
    if out.is_empty() {
        return Err(ChatError::Invalid(
            "The archive has no readable text files.".into(),
        ));
    }
    if !skipped.is_empty() {
        out.push_str(&format!(
            "(Not read: {})",
            skipped
                .iter()
                .take(30)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(out)
}

/// Moves a document to another library, its text and name kept.
pub fn move_document(
    config_dir: &Path,
    from: &str,
    document_id: &str,
    to: &str,
) -> Result<(), ChatError> {
    if from == to {
        return Ok(());
    }
    let source = load(config_dir, from)?;
    let doc = source
        .documents
        .iter()
        .find(|d| d.id == document_id)
        .ok_or_else(|| ChatError::Invalid("That document is gone.".into()))?
        .clone();
    let text = crate::chat::at_rest::read_to_string(
        docs_dir(config_dir, from)?.join(format!("{}.txt", doc.id)),
    )
    .map_err(ChatError::internal)?;
    load(config_dir, to)?;
    add_text(config_dir, to, &doc.name, &text)?;
    remove(config_dir, from, document_id).map(|_| ())
}

/// The text of a file with its line breaks, by its kind: PDF, Office, HTML or plain text.
pub fn read_text(path: &Path) -> Result<String, ChatError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = match ext.as_str() {
        "pdf" => {
            let pattern = glob::Pattern::escape(&path.to_string_lossy());
            let first = rig_core::loaders::PdfFileLoader::with_glob(&pattern)
                .map_err(ChatError::internal)?
                .read()
                .into_iter()
                .next();
            first
                .ok_or_else(|| ChatError::Invalid("The PDF could not be read.".into()))?
                .map_err(|e| ChatError::Invalid(format!("The PDF could not be read: {e}")))?
        }
        e if OFFICE_EXTENSIONS.contains(&e) => office_text(
            &crate::chat::at_rest::read(path).map_err(ChatError::internal)?,
            e,
        )?,
        "html" | "htm" => {
            let raw = crate::chat::at_rest::read_to_string(path).map_err(ChatError::internal)?;
            let doc = scraper::Html::parse_document(&raw);
            let body = scraper::Selector::parse("body").expect("selector");
            doc.select(&body)
                .next()
                .map(|b| b.text().collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
        }
        _ => {
            let bytes = crate::chat::at_rest::read(path).map_err(ChatError::internal)?;
            if bytes.iter().take(8000).any(|b| *b == 0) {
                return Err(ChatError::Invalid(format!("{} is not a text document (PDF, Word, PowerPoint, OpenDocument, HTML, Markdown, text, code and data files are supported).", path.display())));
            }
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    Ok(text)
}

/// The text of a file, by its kind, with whitespace collapsed for indexing.
pub(in crate::chat::grounding) fn extract(path: &Path) -> Result<String, ChatError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = read_text(path)?;
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        return Err(if ext == "pdf" {
            ChatError::Invalid(format!("{} has no text layer (a scan?). Add its pages as images instead; they are read by your model.", path.display()))
        } else {
            ChatError::Invalid(format!("{} has no readable text.", path.display()))
        });
    }
    if text.chars().count() > MAX_DOC_CHARS {
        return Err(ChatError::Invalid(format!(
            "{} is too large to index.",
            path.display()
        )));
    }
    Ok(text)
}
