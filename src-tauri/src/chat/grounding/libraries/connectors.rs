//! Connectors indexed into a library: the documents an MCP server offers, fetched and kept current.

use super::*;

// Connectors indexed into a library (Onyx's connectors): the documents an MCP server offers as
// resources (pages of a wiki, files of a drive, tickets) are read into the library and kept in
// step: new ones added, changed ones re-read, gone ones removed.

/// `source` of a document read from a connector: `mcp:<connector id>:<resource uri>`.
pub const CONNECTOR_SOURCE: &str = "mcp:";
/// Documents read per connector, at most.
pub(super) const CONNECTOR_MAX_DOCS: usize = 500;

/// A fingerprint of a document's text, to see that it changed.
pub(in crate::chat::grounding) fn text_fingerprint(text: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish() as i64
}

/// Brings the documents a library mirrors from its connectors in line with what they offer.
/// A connector that cannot be reached keeps its documents until it answers again.
pub async fn sync_connectors(
    config_dir: &Path,
    library_id: &str,
    connectors: &[std::sync::Arc<dyn crate::tools::registry::Connector>],
) -> Result<SyncReport, String> {
    let mut library = load(config_dir, library_id)?;
    let docs = docs_dir(config_dir, library_id)?;
    std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;
    let mut report = SyncReport::default();
    // What each connector offers now; `None` for one that did not answer.
    let mut offered: std::collections::HashMap<String, Option<Vec<(String, String)>>> =
        std::collections::HashMap::new();
    for id in &library.connectors {
        let Some(connector) = connectors.iter().find(|c| c.id() == id) else {
            report.errors.push(format!("{id} is not connected."));
            offered.insert(id.clone(), None);
            continue;
        };
        match connector.resources().await {
            Ok(list) => {
                let items = list
                    .iter()
                    .filter_map(|r| {
                        let uri = r.get("uri")?.as_str()?.to_string();
                        let name = r
                            .get("title")
                            .or_else(|| r.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or(&uri)
                            .to_string();
                        Some((uri, name))
                    })
                    .take(CONNECTOR_MAX_DOCS)
                    .collect();
                offered.insert(id.clone(), Some(items));
            }
            Err(e) => {
                report
                    .errors
                    .push(format!("{}: {e}", connector.descriptor().display_name));
                offered.insert(id.clone(), None);
            }
        }
    }
    let source_of = |connector: &str, uri: &str| format!("{CONNECTOR_SOURCE}{connector}:{uri}");
    // Documents of connectors no longer mirrored, or no longer offered, leave.
    let mut keep = Vec::new();
    for doc in std::mem::take(&mut library.documents) {
        let Some(source) = doc
            .source
            .clone()
            .filter(|s| s.starts_with(CONNECTOR_SOURCE))
        else {
            keep.push(doc);
            continue;
        };
        let still = library.connectors.iter().any(|c| match offered.get(c) {
            Some(None) => source.starts_with(&source_of(c, "")),
            Some(Some(items)) => items.iter().any(|(uri, _)| source == source_of(c, uri)),
            None => false,
        });
        if still {
            keep.push(doc);
        } else {
            drop_document_files(&docs, &doc.id);
            report.removed += 1;
        }
    }
    library.documents = keep;
    for (connector_id, items) in &offered {
        let (Some(items), Some(connector)) =
            (items, connectors.iter().find(|c| c.id() == connector_id))
        else {
            continue;
        };
        for (uri, name) in items {
            let source = source_of(connector_id, uri);
            let text = match connector.read_resource(uri).await {
                Ok(t) if !t.trim().is_empty() => t,
                Ok(_) => continue,
                Err(e) => {
                    report.errors.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let text: String = crate::support::text::clip(&text, MAX_DOC_CHARS);
            let fingerprint = text_fingerprint(&text);
            if let Some(i) = library
                .documents
                .iter()
                .position(|d| d.source.as_deref() == Some(source.as_str()))
            {
                if library.documents[i].modified == Some(fingerprint) {
                    continue;
                }
                let old = library.documents.remove(i);
                drop_document_files(&docs, &old.id);
                report.updated += 1;
            } else {
                report.added += 1;
            }
            let doc = Document {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                chars: text.chars().count(),
                added_at: Utc::now(),
                source: Some(source),
                modified: Some(fingerprint),
            };
            crate::chat::at_rest::write(docs.join(format!("{}.txt", doc.id)), &text)
                .map_err(|e| e.to_string())?;
            library.documents.push(doc);
        }
    }
    library.synced_at = Some(Utc::now());
    save(config_dir, &library)?;
    Ok(report)
}

/// Mirrors a connector's documents into a library.
pub fn set_connector(
    config_dir: &Path,
    library_id: &str,
    connector_id: &str,
    on: bool,
) -> Result<Library, String> {
    let mut library = load(config_dir, library_id)?;
    library.connectors.retain(|c| c != connector_id);
    if on {
        library.connectors.push(connector_id.to_string());
    }
    save(config_dir, &library)?;
    Ok(library)
}
