//! Qdrant as the vector store, over its REST API.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Value};

use super::{libraries, read_cache, EmbeddingSettings, Passage};

/// Points sent per upsert.
const UPSERT_BATCH: usize = 128;

async fn key() -> String {
    let Ok(account) = crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT) else {
        return String::new();
    };
    crate::tools::auth::keychain_secrets(account)
        .load(super::QDRANT_SECRET)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// One collection per embedding model, since each has its own width and meaning.
pub(in crate::chat::grounding) fn collection(model: &str) -> String {
    let tag: String = model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    format!("cortex_{tag}")
}

/// A stable point id for a passage (FNV-1a, below 2^63 so every JSON reader keeps it exact).
pub(in crate::chat::grounding) fn point_id(library: &str, document: &str, passage: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{library}/{document}/{passage}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h & (u64::MAX >> 1)
}

async fn call(
    settings: &EmbeddingSettings,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let client = crate::support::http::client(std::time::Duration::from_secs(60))?;
    let mut request = client.request(method, format!("{}{path}", settings.qdrant_url));
    let key = key().await;
    if !key.is_empty() {
        request = request.header("api-key", key);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Qdrant did not answer at {}: {e}", settings.qdrant_url))?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    Ok((status, body))
}

fn failed(what: &str, status: u16, body: &Value) -> String {
    let detail = body
        .pointer("/status/error")
        .and_then(Value::as_str)
        .map_or_else(|| body.to_string(), str::to_string);
    format!(
        "Qdrant could not {what} ({status}): {}",
        detail.chars().take(200).collect::<String>()
    )
}

/// Creates the collection for `width`-wide vectors unless it is there; a collection made for
/// another width is an error (the model changed under the same name).
async fn ensure(settings: &EmbeddingSettings, name: &str, width: usize) -> Result<(), String> {
    let (status, body) = call(
        settings,
        reqwest::Method::GET,
        &format!("/collections/{name}"),
        None,
    )
    .await?;
    if status == 200 {
        let size = body
            .pointer("/result/config/params/vectors/size")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if size != 0 && size != width {
            return Err(format!("Qdrant's collection {name} holds {size}-wide vectors, but the model makes {width}-wide ones. Delete the collection in Qdrant to start over."));
        }
        return Ok(());
    }
    if status != 404 {
        return Err(failed("read the collection", status, &body));
    }
    let (status, body) = call(
        settings,
        reqwest::Method::PUT,
        &format!("/collections/{name}"),
        Some(json!({ "vectors": { "size": width, "distance": "Cosine" } })),
    )
    .await?;
    if status != 200 {
        return Err(failed("create the collection", status, &body));
    }
    // Filters on library and document ids are fast with keyword indexes.
    for field in ["library_id", "document_id"] {
        let _ = call(
            settings,
            reqwest::Method::PUT,
            &format!("/collections/{name}/index?wait=true"),
            Some(json!({ "field_name": field, "field_schema": "keyword" })),
        )
        .await;
    }
    Ok(())
}

/// What is in Qdrant, as Cortex last left it: per collection, `library/document/passages`.
fn marker_path(config_dir: &Path) -> std::path::PathBuf {
    crate::chat::store::chat_dir(config_dir).join("qdrant-sync.json")
}

type Marker = BTreeMap<String, BTreeSet<String>>;

fn read_marker(config_dir: &Path) -> Marker {
    crate::chat::store::read_json(&marker_path(config_dir)).unwrap_or_default()
}

/// Brings the collection in line with the libraries: new and changed documents are added,
/// removed ones (and removed libraries) are deleted. Only documents already embedded are
/// sent; the vectors come from the local cache.
pub(in crate::chat::grounding) async fn sync(
    config_dir: &Path,
    settings: &EmbeddingSettings,
) -> Result<(), String> {
    let name = collection(&settings.model);
    let mut marker = read_marker(config_dir);
    let known = marker
        .get(&format!("{}|{name}", settings.qdrant_url))
        .cloned()
        .unwrap_or_default();
    let mut now: BTreeSet<String> = BTreeSet::new();
    let mut library_ids = Vec::new();
    let mut pending: Vec<Value> = Vec::new();
    let mut width = 0;
    let mut changed_docs: Vec<(String, String)> = Vec::new();
    for library in libraries::list(config_dir) {
        library_ids.push(library.id.clone());
        let doc_dir = libraries::docs_dir(config_dir, &library.id)?;
        for document in &library.documents {
            let Ok(text) =
                crate::chat::at_rest::read_to_string(doc_dir.join(format!("{}.txt", document.id)))
            else {
                continue;
            };
            let passages = libraries::passages(&text);
            let Some(vectors) = read_cache(&doc_dir, &document.id, &settings.model, passages.len())
            else {
                continue;
            };
            let stamp = format!("{}/{}/{}", library.id, document.id, passages.len());
            now.insert(stamp.clone());
            if known.contains(&stamp) {
                continue;
            }
            changed_docs.push((library.id.clone(), document.id.clone()));
            for (i, (passage, vector)) in passages.into_iter().zip(vectors).enumerate() {
                width = vector.len();
                pending.push(json!({
                    "id": point_id(&library.id, &document.id, i),
                    "vector": vector,
                    "payload": { "library_id": library.id, "document_id": document.id, "passage": i, "text": passage },
                }));
            }
        }
    }
    if now == known {
        return Ok(());
    }
    if width > 0 {
        ensure(settings, &name, width).await?;
    } else {
        // Nothing to add: only deletions, which need the collection to exist.
        let (status, _) = call(
            settings,
            reqwest::Method::GET,
            &format!("/collections/{name}"),
            None,
        )
        .await?;
        if status == 404 {
            marker.insert(format!("{}|{name}", settings.qdrant_url), now);
            return crate::chat::store::write_json(&marker_path(config_dir), &marker)
                .map_err(String::from);
        }
    }
    // A changed document's old passages go first (it may now have fewer).
    for (library, document) in &changed_docs {
        let filter = json!({ "must": [{ "key": "library_id", "match": { "value": library } }, { "key": "document_id", "match": { "value": document } }] });
        let (status, body) = call(
            settings,
            reqwest::Method::POST,
            &format!("/collections/{name}/points/delete?wait=true"),
            Some(json!({ "filter": filter })),
        )
        .await?;
        if status != 200 {
            return Err(failed("remove a document's old passages", status, &body));
        }
    }
    for batch in pending.chunks(UPSERT_BATCH) {
        let (status, body) = call(
            settings,
            reqwest::Method::PUT,
            &format!("/collections/{name}/points?wait=true"),
            Some(json!({ "points": batch })),
        )
        .await?;
        if status != 200 {
            return Err(failed("store passages", status, &body));
        }
    }
    // Documents and libraries that are gone.
    let gone: BTreeSet<(String, String)> = known
        .difference(&now)
        .filter_map(|s| {
            let mut parts = s.splitn(3, '/');
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect();
    let still: BTreeSet<(String, String)> = now
        .iter()
        .filter_map(|s| {
            let mut parts = s.splitn(3, '/');
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect();
    for (library, document) in gone.difference(&still) {
        let filter = json!({ "must": [{ "key": "library_id", "match": { "value": library } }, { "key": "document_id", "match": { "value": document } }] });
        let (status, body) = call(
            settings,
            reqwest::Method::POST,
            &format!("/collections/{name}/points/delete?wait=true"),
            Some(json!({ "filter": filter })),
        )
        .await?;
        if status != 200 {
            return Err(failed("remove a deleted document", status, &body));
        }
    }
    marker.insert(format!("{}|{name}", settings.qdrant_url), now);
    crate::chat::store::write_json(&marker_path(config_dir), &marker).map_err(String::from)
}

/// The `k` passages nearest to `vector`, in the given libraries (all when empty).
pub async fn search(
    config_dir: &Path,
    settings: &EmbeddingSettings,
    only: &[String],
    vector: &[f32],
    k: usize,
) -> Result<Vec<Passage>, String> {
    let name = collection(&settings.model);
    let libraries: Vec<_> = libraries::list(config_dir)
        .into_iter()
        .filter(|l| only.is_empty() || only.contains(&l.id))
        .collect();
    if libraries.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<&str> = libraries.iter().map(|l| l.id.as_str()).collect();
    let body = json!({ "vector": vector, "limit": k, "with_payload": true, "filter": { "must": [{ "key": "library_id", "match": { "any": ids } }] } });
    let (status, answer) = call(
        settings,
        reqwest::Method::POST,
        &format!("/collections/{name}/points/search"),
        Some(body),
    )
    .await?;
    if status == 404 {
        return Ok(Vec::new());
    }
    if status != 200 {
        return Err(failed("search", status, &answer));
    }
    Ok(read_hits(&answer, &libraries))
}

/// Hits as passages, named by the libraries' and documents' current names.
pub(in crate::chat::grounding) fn read_hits(
    answer: &Value,
    libraries: &[libraries::Library],
) -> Vec<Passage> {
    answer
        .get("result")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            let payload = hit.get("payload")?;
            let library = libraries.iter().find(|l| {
                Some(l.id.as_str()) == payload.get("library_id").and_then(Value::as_str)
            })?;
            let document = library.documents.iter().find(|d| {
                Some(d.id.as_str()) == payload.get("document_id").and_then(Value::as_str)
            })?;
            Some(Passage {
                library: library.name.clone(),
                document: document.name.clone(),
                passage: payload.get("passage").and_then(Value::as_u64).unwrap_or(0) as usize,
                text: payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                score: hit.get("score").and_then(Value::as_f64).unwrap_or(0.0),
            })
        })
        .collect()
}

/// Checks the server: its version, or why it cannot be used.
pub async fn check(settings: &EmbeddingSettings) -> Result<String, String> {
    let (status, body) = call(settings, reqwest::Method::GET, "/collections", None).await?;
    if status == 401 || status == 403 {
        return Err("Qdrant refused the API key.".into());
    }
    if status != 200 {
        return Err(failed("list collections", status, &body));
    }
    let (_, root) = call(settings, reqwest::Method::GET, "/", None)
        .await
        .unwrap_or((0, Value::Null));
    Ok(root
        .get("version")
        .and_then(Value::as_str)
        .map_or_else(|| "Qdrant".to_string(), |v| format!("Qdrant {v}")))
}
