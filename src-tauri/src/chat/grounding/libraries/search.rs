//! Searching libraries: passages, ranking, and what the prompt says about them.

use super::*;

/// Overlapping passages, cut at whitespace where possible.
pub fn passages(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + PASSAGE).min(chars.len());
        if end < chars.len() {
            if let Some(space) = chars[start + PASSAGE / 2..end]
                .iter()
                .rposition(|c| c.is_whitespace())
            {
                end = start + PASSAGE / 2 + space;
            }
        }
        out.push(
            chars[start..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_string(),
        );
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(OVERLAP).max(start + 1);
    }
    out
}

/// Lowercased words without accents, so "regulação" matches "regulacao".
pub use crate::chat::retrieval::tokens;

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
    }
    let mut docs = Vec::new();
    let mut words = Vec::new();
    for library in list(config_dir)
        .into_iter()
        .filter(|l| only.is_empty() || only.contains(&l.id))
    {
        let Ok(dir) = lib_dir(config_dir, &library.id) else {
            continue;
        };
        for document in &library.documents {
            let Ok(text) = crate::chat::at_rest::read_to_string(
                dir.join("docs").join(format!("{}.txt", document.id)),
            ) else {
                continue;
            };
            for (i, passage) in passages(&text).into_iter().enumerate() {
                words.push(tokens(&passage));
                docs.push(Doc {
                    library: library.name.clone(),
                    document: document.name.clone(),
                    passage: i,
                    text: passage,
                });
            }
        }
    }
    crate::chat::retrieval::Corpus::new(words)
        .rank(&query_terms)
        .into_iter()
        .take(k)
        .map(|(i, score)| {
            let d = &docs[i];
            Passage {
                library: d.library.clone(),
                document: d.document.clone(),
                passage: d.passage,
                text: d.text.clone(),
                score: (score * 100.0).round() / 100.0,
            }
        })
        .collect()
}

/// The system-prompt line naming the libraries the model can search.
pub fn prompt(libraries: &[Library]) -> Option<String> {
    let useful: Vec<&Library> = libraries
        .iter()
        .filter(|l| !l.documents.is_empty())
        .collect();
    if useful.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\nThe user's libraries (search them with `search_library` when a question may be answered \
         by their documents, read a whole document with `read_document` when passages are not enough, and cite \
         the document name like [Doc name]):",
    );
    for l in useful {
        out.push_str(&format!(
            "\n- {} ({} documents){}",
            l.name,
            l.documents.len(),
            if l.description.is_empty() {
                String::new()
            } else {
                format!(": {}", l.description)
            }
        ));
    }
    Some(out)
}
