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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rig_core::providers::openai::wire::{self as dialects};
use rig_core::providers::openai::OpenAIConfig;
use serde::{Deserialize, Serialize};

use super::libraries::{self, Passage};
use super::store::write_json;

pub const SECRET: &str = "embeddings";
pub const RERANK_SECRET: &str = "rerank";
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
}

fn settings_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("embeddings.json")
}

pub fn load(config_dir: &Path) -> EmbeddingSettings {
    crate::chat::at_rest::read_to_string(settings_path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &EmbeddingSettings) -> Result<(), String> {
    let mut s = settings.clone();
    s.base_url = s.base_url.trim().trim_end_matches('/').to_string();
    s.model = s.model.trim().to_string();
    s.rerank_url = s.rerank_url.trim().trim_end_matches('/').to_string();
    s.rerank_model = s.rerank_model.trim().to_string();
    write_json(&settings_path(config_dir), &s)
}

/// Whether semantic search is set up enough to try.
pub fn ready(settings: &EmbeddingSettings) -> bool {
    settings.enabled && !settings.base_url.is_empty() && !settings.model.is_empty()
}

async fn key() -> Result<String, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    Ok(crate::tools::auth::keychain_secrets(account).load(SECRET).await?.unwrap_or_default())
}

/// Vectors for `texts`, in order, from the configured model.
pub async fn embed(settings: &EmbeddingSettings, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
    let config = OpenAIConfig::with_key(&dialects::OPENAI, key().await?).with_base_url(settings.base_url.clone());
    let model = config.client().embedding(settings.model.clone(), None);
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(BATCH) {
        let response = tokio::time::timeout(std::time::Duration::from_secs(120), model.call(chunk.to_vec()))
            .await
            .map_err(|_| "The embedding server did not answer in time.".to_string())?
            .map_err(|e| super::context::short_error(&e.to_string()))?;
        if response.embeddings.len() != chunk.len() {
            return Err(format!("The embedding server returned {} vectors for {} passages.", response.embeddings.len(), chunk.len()));
        }
        out.extend(response.embeddings.into_iter().map(|e| e.vec.into_iter().map(|v| v as f32).collect::<Vec<f32>>()));
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
    let tag: String = model.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).take(60).collect();
    doc_dir.join(format!("{document_id}.{tag}.vec.json"))
}

pub fn read_cache(doc_dir: &Path, document_id: &str, model: &str, passages: usize) -> Option<Vec<Vec<f32>>> {
    let cached: Cached = serde_json::from_str(&crate::chat::at_rest::read_to_string(cache_path(doc_dir, document_id, model)).ok()?).ok()?;
    (cached.model == model && cached.vectors.len() == passages).then_some(cached.vectors)
}

fn write_cache(doc_dir: &Path, document_id: &str, model: &str, vectors: Vec<Vec<f32>>) -> Result<(), String> {
    write_json(&cache_path(doc_dir, document_id, model), &Cached { model: model.into(), vectors })
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
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na.sqrt() * nb.sqrt()) }
}

/// Merges ranked lists by reciprocal rank fusion: a passage high in either list, or fairly high
/// in both, comes first. Passages are the same when library, document and position match.
pub fn fuse(lists: &[Vec<Passage>], k: usize) -> Vec<Passage> {
    let mut scores: HashMap<(String, String, usize), (f64, Passage)> = HashMap::new();
    for list in lists {
        for (rank, p) in list.iter().enumerate() {
            let entry = scores.entry((p.library.clone(), p.document.clone(), p.passage)).or_insert((0.0, p.clone()));
            entry.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut merged: Vec<(f64, Passage)> = scores.into_values().collect();
    merged.sort_by(|a, b| b.0.total_cmp(&a.0));
    merged
        .into_iter()
        .take(k)
        .map(|(score, mut p)| {
            p.score = (score * 10_000.0).round() / 10_000.0;
            p
        })
        .collect()
}

/// Embeds every passage of the given libraries not cached yet; returns how many were embedded.
pub async fn index(config_dir: &Path, settings: &EmbeddingSettings, only: &[String]) -> Result<usize, String> {
    let mut done = 0;
    for library in libraries::list(config_dir).into_iter().filter(|l| only.is_empty() || only.contains(&l.id)) {
        let doc_dir = libraries::docs_dir(config_dir, &library.id)?;
        for document in &library.documents {
            let Ok(text) = crate::chat::at_rest::read_to_string(doc_dir.join(format!("{}.txt", document.id))) else { continue };
            let passages = libraries::passages(&text);
            if passages.is_empty() || read_cache(&doc_dir, &document.id, &settings.model, passages.len()).is_some() {
                continue;
            }
            let vectors = embed(settings, passages.clone()).await?;
            done += vectors.len();
            write_cache(&doc_dir, &document.id, &settings.model, vectors)?;
        }
    }
    Ok(done)
}

/// The passages closest in meaning to `query`, best first.
async fn semantic(config_dir: &Path, settings: &EmbeddingSettings, only: &[String], query: &str, k: usize) -> Result<Vec<Passage>, String> {
    index(config_dir, settings, only).await?;
    let question = embed(settings, vec![query.to_string()]).await?.pop().unwrap_or_default();
    let mut scored: Vec<(f32, Passage)> = Vec::new();
    for library in libraries::list(config_dir).into_iter().filter(|l| only.is_empty() || only.contains(&l.id)) {
        let doc_dir = libraries::docs_dir(config_dir, &library.id)?;
        for document in &library.documents {
            let Ok(text) = crate::chat::at_rest::read_to_string(doc_dir.join(format!("{}.txt", document.id))) else { continue };
            let passages = libraries::passages(&text);
            let Some(vectors) = read_cache(&doc_dir, &document.id, &settings.model, passages.len()) else { continue };
            for (i, (passage, vector)) in passages.into_iter().zip(vectors).enumerate() {
                let similarity = cosine(&question, &vector);
                scored.push((similarity, Passage { library: library.name.clone(), document: document.name.clone(), passage: i, text: passage, score: similarity as f64 }));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(scored.into_iter().take(k).map(|(_, p)| p).collect())
}

/// Library search: hybrid when semantic search is set up, keyword-only otherwise. The note says
/// when semantic search was wanted but could not run.
pub async fn search(config_dir: &Path, only: &[String], query: &str, k: usize) -> (Vec<Passage>, Option<String>) {
    let settings = load(config_dir);
    let reranking = !settings.rerank_url.is_empty() && !settings.rerank_model.is_empty();
    // A reranker gets a wider pool to choose from.
    let pool = if reranking { k * 4 } else { k };
    let keyword = libraries::search(config_dir, only, query, pool * 3);
    let (candidates, mut note) = if !ready(&settings) {
        (keyword.into_iter().take(pool).collect::<Vec<_>>(), None)
    } else {
        match semantic(config_dir, &settings, only, query, pool * 3).await {
            Ok(meaning) => (fuse(&[keyword, meaning], pool), None),
            Err(e) => {
                log::warn!("semantic search failed, keyword search only: {e}");
                (keyword.into_iter().take(pool).collect(), Some(format!("Semantic search could not run ({e}); these are keyword matches.")))
            }
        }
    };
    if !reranking || candidates.len() <= 1 {
        return (candidates.into_iter().take(k).collect(), note);
    }
    let key = rerank_key().await.unwrap_or_default();
    match rerank(&settings, &key, query, &candidates, k).await {
        Ok(order) => (order.into_iter().filter_map(|(i, score)| candidates.get(i).cloned().map(|p| Passage { score: (score * 1000.0).round() / 1000.0, ..p })).collect(), note),
        Err(e) => {
            log::warn!("reranking failed: {e}");
            note.get_or_insert_with(|| format!("Reranking could not run ({e}); these are in search order."));
            (candidates.into_iter().take(k).collect(), note)
        }
    }
}

async fn rerank_key() -> Result<String, String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    Ok(crate::tools::auth::keychain_secrets(account).load(RERANK_SECRET).await?.unwrap_or_default())
}

/// The candidates' order by the reranker: `(index, relevance)`, best first, at most `k`.
pub async fn rerank(settings: &EmbeddingSettings, key: &str, query: &str, candidates: &[Passage], k: usize) -> Result<Vec<(usize, f64)>, String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
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
    let body: serde_json::Value = response.json().await.map_err(|e| format!("the reranker answered {status} without JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("the reranker answered {status}: {}", body.get("error").or_else(|| body.get("message")).map_or_else(|| body.to_string(), |e| e.to_string()).chars().take(200).collect::<String>()));
    }
    parse_rerank(&body, candidates.len(), k)
}

/// Reads `{"results": [{"index", "relevance_score"}]}` (Cohere, Jina, llama.cpp, vLLM) or
/// `[{"index", "score"}]` (TEI).
fn parse_rerank(body: &serde_json::Value, n: usize, k: usize) -> Result<Vec<(usize, f64)>, String> {
    let items = body.get("results").or_else(|| body.get("data")).unwrap_or(body).as_array().ok_or("the reranker's answer has no results")?;
    let mut out: Vec<(usize, f64)> = items
        .iter()
        .filter_map(|r| {
            let index = r.get("index")?.as_u64()? as usize;
            let score = r.get("relevance_score").or_else(|| r.get("score"))?.as_f64()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn p(document: &str, passage: usize) -> Passage {
        Passage { library: "L".into(), document: document.into(), passage, text: format!("{document} {passage}"), score: 0.0 }
    }

    #[test]
    fn rerank_answers_in_both_shapes() {
        let cohere = serde_json::json!({ "results": [{ "index": 2, "relevance_score": 0.91 }, { "index": 0, "relevance_score": 0.12 }, { "index": 9, "relevance_score": 0.99 }] });
        assert_eq!(parse_rerank(&cohere, 3, 5).unwrap(), [(2, 0.91), (0, 0.12)], "out-of-range indexes are dropped");
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
        let settings = EmbeddingSettings { rerank_url: format!("http://{addr}/v1"), rerank_model: "bge-reranker".into(), ..Default::default() };
        let candidates = vec![p("pricing", 0), Passage { text: "termination termination clause".into(), ..p("contract", 3) }, Passage { text: "termination notice".into(), ..p("faq", 1) }];
        let order = rerank(&settings, "", "how do I cancel", &candidates, 2).await.unwrap();
        assert_eq!(order.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1, 2]);
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
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0, "different widths never match");
        let dir = tempfile::tempdir().unwrap();
        write_cache(dir.path(), "doc", "nomic-embed-text:v1.5", vec![vec![0.1, 0.2], vec![0.3, 0.4]]).unwrap();
        assert_eq!(read_cache(dir.path(), "doc", "nomic-embed-text:v1.5", 2).unwrap().len(), 2);
        assert!(read_cache(dir.path(), "doc", "nomic-embed-text:v1.5", 3).is_none(), "a changed document is embedded again");
        assert!(read_cache(dir.path(), "doc", "other-model", 2).is_none(), "each model has its own vectors");
    }

    #[test]
    fn settings_round_trip_and_readiness() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!ready(&load(dir.path())), "off until set up");
        save(dir.path(), &EmbeddingSettings { enabled: true, base_url: " http://localhost:11434/v1/ ".into(), model: "nomic-embed-text".into(), has_key: false, ..Default::default() }).unwrap();
        let s = load(dir.path());
        assert_eq!(s.base_url, "http://localhost:11434/v1");
        assert!(ready(&s));
    }
}
