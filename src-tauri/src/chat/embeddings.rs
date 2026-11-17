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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rig_core::providers::openai::wire::{self as dialects};
use rig_core::providers::openai::OpenAIConfig;
use serde::{Deserialize, Serialize};

use super::libraries::{self, Passage};
use super::store::write_json;

pub const SECRET: &str = "embeddings";
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
}

fn settings_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("embeddings.json")
}

pub fn load(config_dir: &Path) -> EmbeddingSettings {
    std::fs::read_to_string(settings_path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &EmbeddingSettings) -> Result<(), String> {
    let mut s = settings.clone();
    s.base_url = s.base_url.trim().trim_end_matches('/').to_string();
    s.model = s.model.trim().to_string();
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
    let cached: Cached = serde_json::from_str(&std::fs::read_to_string(cache_path(doc_dir, document_id, model)).ok()?).ok()?;
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
            let Ok(text) = std::fs::read_to_string(doc_dir.join(format!("{}.txt", document.id))) else { continue };
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
            let Ok(text) = std::fs::read_to_string(doc_dir.join(format!("{}.txt", document.id))) else { continue };
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
    let keyword = libraries::search(config_dir, only, query, k * 3);
    let settings = load(config_dir);
    if !ready(&settings) {
        return (keyword.into_iter().take(k).collect(), None);
    }
    match semantic(config_dir, &settings, only, query, k * 3).await {
        Ok(meaning) => (fuse(&[keyword, meaning], k), None),
        Err(e) => {
            log::warn!("semantic search failed, keyword search only: {e}");
            (keyword.into_iter().take(k).collect(), Some(format!("Semantic search could not run ({e}); these are keyword matches.")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(document: &str, passage: usize) -> Passage {
        Passage { library: "L".into(), document: document.into(), passage, text: format!("{document} {passage}"), score: 0.0 }
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
        save(dir.path(), &EmbeddingSettings { enabled: true, base_url: " http://localhost:11434/v1/ ".into(), model: "nomic-embed-text".into(), has_key: false }).unwrap();
        let s = load(dir.path());
        assert_eq!(s.base_url, "http://localhost:11434/v1");
        assert!(ready(&s));
    }
}
