//! Ranking tools by meaning, so the ones a request needs are found among many.

use super::*;

/// The deferrable tools ranked by meaning against `query`, when the user set up an embedding
/// model: the registry's semantic search. Tool descriptions are embedded once per process.
pub(super) async fn semantic_tool_ranking(
    config_dir: &std::path::Path,
    offered: &[ChatTool],
    query: &str,
) -> Option<Vec<String>> {
    use crate::chat::embeddings;
    let settings = embeddings::load(config_dir);
    if !embeddings::ready(&settings) {
        return None;
    }
    rank_by_meaning(offered, query, &settings.model, |texts| async {
        embeddings::embed(&settings, texts).await
    })
    .await
}

/// The ranking itself: `embed` turns the query and each tool's name and description into
/// vectors (with `model`'s cache), and tools come back closest in meaning first.
pub(in crate::chat::turn::engine) async fn rank_by_meaning<F, Fut>(
    offered: &[ChatTool],
    query: &str,
    model: &str,
    embed: F,
) -> Option<Vec<String>>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Vec<f32>>, String>>,
{
    use crate::chat::embeddings;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    if query.trim().is_empty() {
        return None;
    }
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let tools: Vec<&ChatTool> = offered.iter().filter(|t| deferred::deferrable(t)).collect();
    let text = |t: &ChatTool| {
        format!(
            "{}: {}",
            t.name.replace("__", " ").replace('_', " "),
            t.description
        )
    };
    let key = |t: &ChatTool| format!("{model}\n{}", text(t));
    let missing: Vec<String> = {
        let cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        tools
            .iter()
            .filter(|t| !cache.contains_key(&key(t)))
            .map(|t| text(t))
            .collect()
    };
    let mut wanted = vec![query.to_string()];
    wanted.extend(missing.iter().cloned());
    let vectors = embed(wanted).await.ok()?;
    let (query_vector, new) = vectors.split_first()?;
    {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        for (text, vector) in missing.iter().zip(new) {
            cache.insert(format!("{model}\n{text}"), vector.clone());
        }
    }
    let cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    let mut scored: Vec<(f32, String)> = tools
        .iter()
        .filter_map(|t| {
            cache
                .get(&key(t))
                .map(|v| (embeddings::cosine(query_vector, v), t.name.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Some(scored.into_iter().map(|(_, name)| name).collect())
}
