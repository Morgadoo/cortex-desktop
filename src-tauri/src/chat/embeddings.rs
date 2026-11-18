//! Semantic search for libraries, as Open WebUI, Onyx and LibreChat do: hybrid retrieval.
//!
//! Keyword search (BM25) finds passages that share the question's words; an embedding model
//! finds the ones that mean the same in other words ("cancel the contract" → "termination
//! clause"). With an embedding model set up, both rank the passages and reciprocal rank fusion
//! merges them. Without one, or when it fails, keyword search answers alone.
//!
//! Any OpenAI-compatible `/embeddings` endpoint works: OpenAI, Mistral, Ollama, LM Studio,
//! llama.cpp, vLLM, LocalAI. Passage vectors are cached next to each document's text, keyed by
//! model, so documents are embedded once. The key lives in the keychain (`embeddings`).
//!
//! A reranker, when set up, reads the question beside each candidate passage and orders them by
//! how well they answer it (Open WebUI's and Onyx's reranking step). Any server with the Cohere
//! and Jina `/rerank` API works: Cohere, Jina, Voyage, llama.cpp's server, vLLM, Infinity, and
//! Hugging Face TEI. Its key is `rerank` in the keychain.
//!
//! The model can also be built in (`base_url` = `builtin`, model = a built-in embedding model's
//! id): a sentence-transformer that runs inside Lumo, so semantic search works offline with
//! nothing to install.
//!
//! Where the vectors are searched (Open WebUI's choice of vector database): on this computer,
//! from the per-document cache (the default; nothing to run), or in a Qdrant server, local or
//! Qdrant Cloud, for large libraries. With Qdrant, vectors are still cached here, so changing
//! stores never embeds again; Qdrant holds a copy of each passage and answers the searches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rig_core::providers::openai::wire::{self as dialects};
use rig_core::providers::openai::OpenAIConfig;
use serde::{Deserialize, Serialize};

use super::libraries::{self, Passage};
use super::store::write_json;

pub const SECRET: &str = "embeddings";
pub const RERANK_SECRET: &str = "rerank";
pub const QDRANT_SECRET: &str = "qdrant";
/// `base_url` for the embedding models built into Lumo.
pub const BUILTIN: &str = "builtin";
/// Passages sent per request.
const BATCH: usize = 64;
/// Reciprocal rank fusion's damping constant; 60 is the usual choice.
const RRF_K: f64 = 60.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EmbeddingSettings {
    pub enabled: bool,
    /// The server's base URL, ending in `/v1`.
    pub base_url: String,
    pub model: String,
    /// Whether a key is saved in the keychain (never the key itself).
    pub has_key: bool,
    /// The reranker's base URL (ending in `/v1` where the API has it); off when empty.
    pub rerank_url: String,
    pub rerank_model: String,
    pub has_rerank_key: bool,
    /// Where vectors are searched: `local` (this computer) or `qdrant`.
    pub vector_store: String,
    /// The Qdrant server, e.g. `http://localhost:6333`.
    pub qdrant_url: String,
    pub has_qdrant_key: bool,
}

impl EmbeddingSettings {
    pub fn uses_qdrant(&self) -> bool {
        self.vector_store == "qdrant" && !self.qdrant_url.is_empty()
    }
}

fn settings_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("embeddings.json")
}

pub fn load(config_dir: &Path) -> EmbeddingSettings {
    crate::chat::at_rest::read_to_string(settings_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &EmbeddingSettings) -> Result<(), String> {
    let mut s = settings.clone();
    s.base_url = s.base_url.trim().trim_end_matches('/').to_string();
    s.model = s.model.trim().to_string();
    s.rerank_url = s.rerank_url.trim().trim_end_matches('/').to_string();
    s.rerank_model = s.rerank_model.trim().to_string();
    s.qdrant_url = s.qdrant_url.trim().trim_end_matches('/').to_string();
    if s.vector_store != "qdrant" {
        s.vector_store = "local".into();
    }
    write_json(&settings_path(config_dir), &s)
}

/// Whether semantic search is set up enough to try.
pub fn ready(settings: &EmbeddingSettings) -> bool {
    settings.enabled && !settings.base_url.is_empty() && !settings.model.is_empty()
}

async fn key() -> Result<String, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    Ok(crate::tools::auth::keychain_secrets(account)
        .load(SECRET)
        .await?
        .unwrap_or_default())
}

/// Vectors for `texts`, in order, from the configured model.
pub async fn embed(
    settings: &EmbeddingSettings,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    if settings.base_url == BUILTIN {
        let root = super::local_models::root()?;
        let model = settings.model.clone();
        return tokio::task::spawn_blocking(move || {
            super::local_models::embed_texts(&root, &model, &texts)
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let config = OpenAIConfig::with_key(&dialects::OPENAI, key().await?)
        .with_base_url(settings.base_url.clone());
    let model = config.client().embedding(settings.model.clone(), None);
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(BATCH) {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            model.call(chunk.to_vec()),
        )
        .await
        .map_err(|_| "The embedding server did not answer in time.".to_string())?
        .map_err(|e| super::context::short_error(&e.to_string()))?;
        if response.embeddings.len() != chunk.len() {
            return Err(format!(
                "The embedding server returned {} vectors for {} passages.",
                response.embeddings.len(),
                chunk.len()
            ));
        }
        out.extend(
            response
                .embeddings
                .into_iter()
                .map(|e| e.vec.into_iter().map(|v| v as f32).collect::<Vec<f32>>()),
        );
    }
    Ok(out)
}

/// A document's passage vectors, for one model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cached {
    pub model: String,
    pub vectors: Vec<Vec<f32>>,
}

fn cache_path(doc_dir: &Path, document_id: &str, model: &str) -> PathBuf {
    let tag: String = model
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(60)
        .collect();
    doc_dir.join(format!("{document_id}.{tag}.vec.json"))
}

pub fn read_cache(
    doc_dir: &Path,
    document_id: &str,
    model: &str,
    passages: usize,
) -> Option<Vec<Vec<f32>>> {
    let cached: Cached = serde_json::from_str(
        &crate::chat::at_rest::read_to_string(cache_path(doc_dir, document_id, model)).ok()?,
    )
    .ok()?;
    (cached.model == model && cached.vectors.len() == passages).then_some(cached.vectors)
}

fn write_cache(
    doc_dir: &Path,
    document_id: &str,
    model: &str,
    vectors: Vec<Vec<f32>>,
) -> Result<(), String> {
    write_json(
        &cache_path(doc_dir, document_id, model),
        &Cached {
            model: model.into(),
            vectors,
        },
    )
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Merges ranked lists by reciprocal rank fusion: a passage high in either list, or fairly high
/// in both, comes first. Passages are the same when library, document and position match.
/// Equal scores (one passage first in one list, another first in the other) go to the passage
/// the first list ranks higher, so the order never depends on chance.
pub fn fuse(lists: &[Vec<Passage>], k: usize) -> Vec<Passage> {
    // A passage: library, document, position.
    type Key = (String, String, usize);
    // Score, then (list, rank) where the passage was first seen, for ties.
    type Scored = (f64, (usize, usize), Passage);
    let mut scores: HashMap<Key, Scored> = HashMap::new();
    for (which, list) in lists.iter().enumerate() {
        for (rank, p) in list.iter().enumerate() {
            let entry = scores
                .entry((p.library.clone(), p.document.clone(), p.passage))
                .or_insert((0.0, (which, rank), p.clone()));
            entry.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut merged: Vec<Scored> = scores.into_values().collect();
    merged.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    merged
        .into_iter()
        .take(k)
        .map(|(score, _, mut p)| {
            p.score = (score * 10_000.0).round() / 10_000.0;
            p
        })
        .collect()
}

/// Embeds every passage of the given libraries not cached yet; returns how many were embedded.
pub async fn index(
    config_dir: &Path,
    settings: &EmbeddingSettings,
    only: &[String],
) -> Result<usize, String> {
    let mut done = 0;
    for library in libraries::list(config_dir)
        .into_iter()
        .filter(|l| only.is_empty() || only.contains(&l.id))
    {
        let doc_dir = libraries::docs_dir(config_dir, &library.id)?;
        for document in &library.documents {
            let Ok(text) =
                crate::chat::at_rest::read_to_string(doc_dir.join(format!("{}.txt", document.id)))
            else {
                continue;
            };
            let passages = libraries::passages(&text);
            if passages.is_empty()
                || read_cache(&doc_dir, &document.id, &settings.model, passages.len()).is_some()
            {
                continue;
            }
            let vectors = embed(settings, passages.clone()).await?;
            done += vectors.len();
            write_cache(&doc_dir, &document.id, &settings.model, vectors)?;
        }
    }
    if settings.uses_qdrant() {
        qdrant::sync(config_dir, settings).await?;
    }
    Ok(done)
}

/// The passages closest in meaning to `query`, best first.
async fn semantic(
    config_dir: &Path,
    settings: &EmbeddingSettings,
    only: &[String],
    query: &str,
    k: usize,
) -> Result<Vec<Passage>, String> {
    index(config_dir, settings, only).await?;
    let question = embed(settings, vec![query.to_string()])
        .await?
        .pop()
        .unwrap_or_default();
    if settings.uses_qdrant() {
        return qdrant::search(config_dir, settings, only, &question, k).await;
    }
    let mut scored: Vec<(f32, Passage)> = Vec::new();
    for library in libraries::list(config_dir)
        .into_iter()
        .filter(|l| only.is_empty() || only.contains(&l.id))
    {
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
            for (i, (passage, vector)) in passages.into_iter().zip(vectors).enumerate() {
                let similarity = cosine(&question, &vector);
                scored.push((
                    similarity,
                    Passage {
                        library: library.name.clone(),
                        document: document.name.clone(),
                        passage: i,
                        text: passage,
                        score: similarity as f64,
                    },
                ));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(scored.into_iter().take(k).map(|(_, p)| p).collect())
}

/// Library search: hybrid when semantic search is set up, keyword-only otherwise. The note says
/// when semantic search was wanted but could not run.
pub async fn search(
    config_dir: &Path,
    only: &[String],
    query: &str,
    k: usize,
) -> (Vec<Passage>, Option<String>) {
    let settings = load(config_dir);
    let reranking = !settings.rerank_url.is_empty() && !settings.rerank_model.is_empty();
    // A reranker gets a wider pool to choose from.
    let pool = if reranking { k * 4 } else { k };
    let keyword = libraries::search(config_dir, only, query, pool * 3);
    let (candidates, mut note) = if !ready(&settings) {
        (keyword.into_iter().take(pool).collect::<Vec<_>>(), None)
    } else {
        match semantic(config_dir, &settings, only, query, pool * 3).await {
            // Meaning first: it decides ties, which come from a passage that only shares
            // common words with the question.
            Ok(meaning) => (fuse(&[meaning, keyword], pool), None),
            Err(e) => {
                log::warn!("semantic search failed, keyword search only: {e}");
                (
                    keyword.into_iter().take(pool).collect(),
                    Some(format!(
                        "Semantic search could not run ({e}); these are keyword matches."
                    )),
                )
            }
        }
    };
    if !reranking || candidates.len() <= 1 {
        return (candidates.into_iter().take(k).collect(), note);
    }
    let key = rerank_key().await.unwrap_or_default();
    match rerank(&settings, &key, query, &candidates, k).await {
        Ok(order) => (
            order
                .into_iter()
                .filter_map(|(i, score)| {
                    candidates.get(i).cloned().map(|p| Passage {
                        score: (score * 1000.0).round() / 1000.0,
                        ..p
                    })
                })
                .collect(),
            note,
        ),
        Err(e) => {
            log::warn!("reranking failed: {e}");
            note.get_or_insert_with(|| {
                format!("Reranking could not run ({e}); these are in search order.")
            });
            (candidates.into_iter().take(k).collect(), note)
        }
    }
}

async fn rerank_key() -> Result<String, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    Ok(crate::tools::auth::keychain_secrets(account)
        .load(RERANK_SECRET)
        .await?
        .unwrap_or_default())
}

/// The candidates' order by the reranker: `(index, relevance)`, best first, at most `k`.
pub async fn rerank(
    settings: &EmbeddingSettings,
    key: &str,
    query: &str,
    candidates: &[Passage],
    k: usize,
) -> Result<Vec<(usize, f64)>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let documents: Vec<&str> = candidates.iter().map(|p| p.text.as_str()).collect();
    let url = format!("{}/rerank", settings.rerank_url.trim_end_matches('/'));
    let mut request = client.post(&url).json(&serde_json::json!({
        "model": settings.rerank_model,
        "query": query,
        "documents": documents,
        // Hugging Face TEI reads `texts`.
        "texts": documents,
        "top_n": k,
    }));
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("the reranker answered {status} without JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "the reranker answered {status}: {}",
            body.get("error")
                .or_else(|| body.get("message"))
                .map_or_else(|| body.to_string(), |e| e.to_string())
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    parse_rerank(&body, candidates.len(), k)
}

/// Reads `{"results": [{"index", "relevance_score"}]}` (Cohere, Jina, llama.cpp, vLLM) or
/// `[{"index", "score"}]` (TEI).
fn parse_rerank(body: &serde_json::Value, n: usize, k: usize) -> Result<Vec<(usize, f64)>, String> {
    let items = body
        .get("results")
        .or_else(|| body.get("data"))
        .unwrap_or(body)
        .as_array()
        .ok_or("the reranker's answer has no results")?;
    let mut out: Vec<(usize, f64)> = items
        .iter()
        .filter_map(|r| {
            let index = r.get("index")?.as_u64()? as usize;
            let score = r
                .get("relevance_score")
                .or_else(|| r.get("score"))?
                .as_f64()?;
            (index < n).then_some((index, score))
        })
        .collect();
    if out.is_empty() {
        return Err("the reranker returned no scores".into());
    }
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by_key(|(i, _)| *i);
    out.truncate(k);
    Ok(out)
}

/// Qdrant as the vector store, over its REST API.
pub mod qdrant {
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
    pub fn collection(model: &str) -> String {
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
        format!("lumo_{tag}")
    }

    /// A stable point id for a passage (FNV-1a, below 2^63 so every JSON reader keeps it exact).
    pub fn point_id(library: &str, document: &str, passage: usize) -> u64 {
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
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

    /// What is in Qdrant, as Lumo last left it: per collection, `library/document/passages`.
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
    pub async fn sync(config_dir: &Path, settings: &EmbeddingSettings) -> Result<(), String> {
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
                let Ok(text) = crate::chat::at_rest::read_to_string(
                    doc_dir.join(format!("{}.txt", document.id)),
                ) else {
                    continue;
                };
                let passages = libraries::passages(&text);
                let Some(vectors) =
                    read_cache(&doc_dir, &document.id, &settings.model, passages.len())
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
                return crate::chat::store::write_json(&marker_path(config_dir), &marker);
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
        crate::chat::store::write_json(&marker_path(config_dir), &marker)
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
    pub fn read_hits(answer: &Value, libraries: &[libraries::Library]) -> Vec<Passage> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(document: &str, passage: usize) -> Passage {
        Passage {
            library: "L".into(),
            document: document.into(),
            passage,
            text: format!("{document} {passage}"),
            score: 0.0,
        }
    }

    #[test]
    fn rerank_answers_in_both_shapes() {
        let cohere = serde_json::json!({ "results": [{ "index": 2, "relevance_score": 0.91 }, { "index": 0, "relevance_score": 0.12 }, { "index": 9, "relevance_score": 0.99 }] });
        assert_eq!(
            parse_rerank(&cohere, 3, 5).unwrap(),
            [(2, 0.91), (0, 0.12)],
            "out-of-range indexes are dropped"
        );
        let tei = serde_json::json!([{ "index": 0, "score": 0.2 }, { "index": 1, "score": 0.8 }]);
        assert_eq!(parse_rerank(&tei, 2, 1).unwrap(), [(1, 0.8)]);
        assert!(parse_rerank(&serde_json::json!({ "results": [] }), 2, 1).is_err());
    }

    #[tokio::test]
    async fn a_reranker_reorders_passages() {
        // A stand-in reranker that scores passages by how many times they say "termination".
        let app = axum::Router::new().route(
            "/v1/rerank",
            axum::routing::post(|axum::Json(body): axum::Json<serde_json::Value>| async move {
                let docs = body["documents"].as_array().cloned().unwrap_or_default();
                let results: Vec<serde_json::Value> = docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| serde_json::json!({ "index": i, "relevance_score": d.as_str().unwrap_or("").matches("termination").count() as f64 }))
                    .collect();
                axum::Json(serde_json::json!({ "results": results }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let settings = EmbeddingSettings {
            rerank_url: format!("http://{addr}/v1"),
            rerank_model: "bge-reranker".into(),
            ..Default::default()
        };
        let candidates = vec![
            p("pricing", 0),
            Passage {
                text: "termination termination clause".into(),
                ..p("contract", 3)
            },
            Passage {
                text: "termination notice".into(),
                ..p("faq", 1)
            },
        ];
        let order = rerank(&settings, "", "how do I cancel", &candidates, 2)
            .await
            .unwrap();
        assert_eq!(order.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn fusion_breaks_ties_by_the_first_list() {
        // "x" leads by meaning, "y" by keywords: equal scores, every time the same order.
        let meaning = vec![p("x", 0), p("y", 0)];
        let keyword = vec![p("y", 0), p("x", 0)];
        for _ in 0..50 {
            let fused = fuse(&[meaning.clone(), keyword.clone()], 2);
            assert_eq!(fused[0].score, fused[1].score);
            assert_eq!(
                (fused[0].document.as_str(), fused[1].document.as_str()),
                ("x", "y")
            );
        }
    }

    #[test]
    fn fusion_rewards_agreement() {
        // "b" is second by keywords and first by meaning: it wins over each list's other top hit.
        let keyword = vec![p("a", 0), p("b", 0), p("c", 0)];
        let meaning = vec![p("b", 0), p("d", 0), p("a", 1)];
        let fused = fuse(&[keyword, meaning], 3);
        assert_eq!(fused[0].document, "b");
        assert_eq!(fused.len(), 3);
        assert!(fused[0].score > fused[1].score);
    }

    #[test]
    fn cosine_and_cache() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(
            cosine(&[1.0], &[1.0, 2.0]),
            0.0,
            "different widths never match"
        );
        let dir = tempfile::tempdir().unwrap();
        write_cache(
            dir.path(),
            "doc",
            "nomic-embed-text:v1.5",
            vec![vec![0.1, 0.2], vec![0.3, 0.4]],
        )
        .unwrap();
        assert_eq!(
            read_cache(dir.path(), "doc", "nomic-embed-text:v1.5", 2)
                .unwrap()
                .len(),
            2
        );
        assert!(
            read_cache(dir.path(), "doc", "nomic-embed-text:v1.5", 3).is_none(),
            "a changed document is embedded again"
        );
        assert!(
            read_cache(dir.path(), "doc", "other-model", 2).is_none(),
            "each model has its own vectors"
        );
    }

    #[test]
    fn qdrant_ids_and_hits() {
        assert_eq!(qdrant::collection("minilm-l6"), "lumo_minilm_l6");
        let a = qdrant::point_id("lib", "doc", 0);
        assert_eq!(a, qdrant::point_id("lib", "doc", 0), "stable");
        assert_ne!(a, qdrant::point_id("lib", "doc", 1));
        assert!(a < 1 << 63);
        let library: libraries::Library = serde_json::from_value(serde_json::json!({ "id": "L1", "name": "Contracts", "documents": [{ "id": "D1", "name": "lease.pdf", "chars": 10, "addedAt": "2026-01-01T00:00:00Z" }], "createdAt": "2026-01-01T00:00:00Z" })).unwrap();
        let answer = serde_json::json!({ "result": [
            { "id": 1, "score": 0.81, "payload": { "library_id": "L1", "document_id": "D1", "passage": 3, "text": "Termination clause" } },
            { "id": 2, "score": 0.5, "payload": { "library_id": "L1", "document_id": "gone", "passage": 0, "text": "x" } },
        ] });
        let hits = qdrant::read_hits(&answer, &[library]);
        assert_eq!(hits.len(), 1, "passages of removed documents are dropped");
        assert_eq!(
            (
                hits[0].library.as_str(),
                hits[0].document.as_str(),
                hits[0].passage
            ),
            ("Contracts", "lease.pdf", 3)
        );
    }

    /// With a Qdrant server at `LUMO_TEST_QDRANT` (e.g. http://localhost:6333) and the built-in
    /// MiniLM downloaded under `LUMO_TEST_MODELS`: libraries are embedded on this computer,
    /// stored in Qdrant, found by meaning, and removed documents leave Qdrant too.
    #[tokio::test]
    #[ignore]
    async fn a_qdrant_server_stores_and_finds_passages() {
        let (Ok(url), Ok(models)) = (
            std::env::var("LUMO_TEST_QDRANT"),
            std::env::var("LUMO_TEST_MODELS"),
        ) else {
            return;
        };
        super::super::local_models::init(Path::new(&models));
        let dir = tempfile::tempdir().unwrap();
        let settings = EmbeddingSettings {
            enabled: true,
            base_url: BUILTIN.into(),
            model: "minilm-l6".into(),
            qdrant_url: url,
            vector_store: "qdrant".into(),
            ..Default::default()
        };
        save(dir.path(), &settings).unwrap();
        assert!(qdrant::check(&settings)
            .await
            .unwrap()
            .starts_with("Qdrant"));
        let lib = libraries::create(dir.path(), "Contracts", "").unwrap();
        libraries::add_text(dir.path(), &lib.id, "lease.txt", "Either party may end this agreement by giving thirty days written notice to the other party.").unwrap();
        libraries::add_text(
            dir.path(),
            &lib.id,
            "bakery.txt",
            "Fresh sourdough loaves are baked every Saturday morning and sold until noon.",
        )
        .unwrap();
        let other = libraries::create(dir.path(), "Recipes", "").unwrap();
        libraries::add_text(
            dir.path(),
            &other.id,
            "soup.txt",
            "Simmer the lentils with cumin and garlic for forty minutes.",
        )
        .unwrap();
        assert_eq!(index(dir.path(), &settings, &[]).await.unwrap(), 3);
        let question = embed(&settings, vec!["How can I cancel the contract?".into()])
            .await
            .unwrap()
            .pop()
            .unwrap();
        let hits = qdrant::search(dir.path(), &settings, &[], &question, 3)
            .await
            .unwrap();
        eprintln!("{hits:?}");
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0].document, "lease.txt",
            "found by meaning: no word in common"
        );
        let only = qdrant::search(
            dir.path(),
            &settings,
            std::slice::from_ref(&other.id),
            &question,
            3,
        )
        .await
        .unwrap();
        assert!(
            only.iter().all(|p| p.library == "Recipes"),
            "limited to the libraries asked"
        );
        // Through the app's search: semantic and keyword fused.
        let (fused, note) = search(dir.path(), &[], "How can I cancel the contract?", 2).await;
        assert!(note.is_none(), "{note:?}");
        assert_eq!(fused[0].document, "lease.txt");
        // Removing a document removes its passages from Qdrant.
        let lease = libraries::load(dir.path(), &lib.id)
            .unwrap()
            .documents
            .into_iter()
            .find(|d| d.name == "lease.txt")
            .unwrap();
        libraries::remove(dir.path(), &lib.id, &lease.id).unwrap();
        index(dir.path(), &settings, &[]).await.unwrap();
        let after = qdrant::search(dir.path(), &settings, &[], &question, 3)
            .await
            .unwrap();
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|p| p.document != "lease.txt"));
        libraries::delete(dir.path(), &other.id).unwrap();
        index(dir.path(), &settings, &[]).await.unwrap();
        let (_, raw) = (0, reqwest::Client::new().post(format!("{}/collections/{}/points/count", settings.qdrant_url, qdrant::collection("minilm-l6"))).json(&serde_json::json!({"exact": true, "filter": { "must": [{ "key": "library_id", "match": { "any": [lib.id, other.id] } }] }})).send().await.unwrap().json::<serde_json::Value>().await.unwrap());
        assert_eq!(
            raw.pointer("/result/count")
                .and_then(serde_json::Value::as_u64),
            Some(1),
            "a deleted library's passages are gone too: {raw}"
        );
    }

    #[test]
    fn settings_round_trip_and_readiness() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!ready(&load(dir.path())), "off until set up");
        save(
            dir.path(),
            &EmbeddingSettings {
                enabled: true,
                base_url: " http://localhost:11434/v1/ ".into(),
                model: "nomic-embed-text".into(),
                has_key: false,
                ..Default::default()
            },
        )
        .unwrap();
        let s = load(dir.path());
        assert_eq!(s.base_url, "http://localhost:11434/v1");
        assert!(ready(&s));
    }
}
