//! Knowledge pages: an agentic knowledge base, as in Vibe's.
//!
//! Short facts (memory.rs) ride in every prompt. Pages are longer: notes about a project, a
//! client, how the user likes reports written. They are recalled only when relevant: the pages
//! that best match the user's message (BM25 over title and body) go into the prompt, the rest are
//! listed by title so the model can read or search them. The model writes and updates pages with
//! tools, the user edits them on the Knowledge page, and answers cite the pages they used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::libraries::tokens;
use super::store::write_json;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub title: String,
    /// Markdown.
    pub body: String,
    /// `user` or `assistant`: who wrote the latest version.
    pub author: String,
    /// Where the content came from: conversation titles, documents or URLs.
    #[serde(default)]
    pub sources: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Pages kept at most.
pub const MAX_PAGES: usize = 300;
/// Characters of a page's body.
pub const MAX_BODY: usize = 20_000;
/// Pages recalled into a prompt, and the characters of each.
const RECALL_PAGES: usize = 3;
const RECALL_CHARS: usize = 3_000;

fn path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("knowledge.json")
}

pub fn load(config_dir: &Path) -> Vec<Page> {
    crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_all(config_dir: &Path, pages: &[Page]) -> Result<(), String> {
    write_json(&path(config_dir), &pages)
}

/// Finds a page by id, id prefix (8+ characters) or exact title (any case).
pub fn find<'a>(pages: &'a [Page], key: &str) -> Option<&'a Page> {
    let key = key.trim();
    pages
        .iter()
        .find(|p| p.id == key)
        .or_else(|| (key.len() >= 8).then(|| pages.iter().find(|p| p.id.starts_with(key))).flatten())
        .or_else(|| pages.iter().find(|p| p.title.eq_ignore_ascii_case(key)))
}

/// Creates a page, or replaces the body of the one with this id or title.
pub fn write(config_dir: &Path, id: Option<&str>, title: &str, body: &str, author: &str, sources: &[String]) -> Result<Page, String> {
    let title: String = title.trim().chars().take(120).collect();
    if title.is_empty() {
        return Err("A knowledge page needs a title.".into());
    }
    let body: String = body.trim().chars().take(MAX_BODY).collect();
    let mut pages = load(config_dir);
    let existing = id
        .and_then(|i| find(&pages, i).map(|p| p.id.clone()))
        .or_else(|| find(&pages, &title).map(|p| p.id.clone()));
    let page = match existing.and_then(|id| pages.iter_mut().find(|p| p.id == id)) {
        Some(page) => {
            page.title = title;
            page.body = body;
            page.author = author.into();
            for s in sources {
                if !s.trim().is_empty() && !page.sources.contains(s) {
                    page.sources.push(s.trim().to_string());
                }
            }
            page.updated_at = Utc::now();
            page.clone()
        }
        None => {
            if pages.len() >= MAX_PAGES {
                return Err(format!("The knowledge base is full ({MAX_PAGES} pages). Remove some first."));
            }
            let page = Page {
                id: uuid::Uuid::new_v4().to_string(),
                title,
                body,
                author: author.into(),
                sources: sources.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                updated_at: Utc::now(),
            };
            pages.push(page.clone());
            page
        }
    };
    save_all(config_dir, &pages)?;
    Ok(page)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Page>, String> {
    let mut pages = load(config_dir);
    pages.retain(|p| p.id != id);
    save_all(config_dir, &pages)?;
    Ok(pages)
}

/// Pages ranked by BM25 against `query`, best first, with their scores; the title counts twice.
pub fn rank<'a>(pages: &'a [Page], query: &str) -> Vec<(&'a Page, f64)> {
    let query_terms = tokens(query);
    if query_terms.is_empty() || pages.is_empty() {
        return Vec::new();
    }
    let docs: Vec<(HashMap<String, f64>, f64)> = pages
        .iter()
        .map(|p| {
            let mut words = tokens(&p.title);
            words.extend(tokens(&p.title));
            words.extend(tokens(&p.body));
            let mut terms = HashMap::new();
            for w in &words {
                *terms.entry(w.clone()).or_insert(0.0) += 1.0;
            }
            (terms, words.len() as f64)
        })
        .collect();
    let n = docs.len() as f64;
    let avg = docs.iter().map(|d| d.1).sum::<f64>() / n;
    let (k1, b) = (1.5, 0.75);
    let mut scored: Vec<(&Page, f64)> = pages
        .iter()
        .zip(&docs)
        .map(|(page, (terms, len))| {
            let score = query_terms
                .iter()
                .map(|q| {
                    let df = docs.iter().filter(|d| d.0.contains_key(q)).count() as f64;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    let tf = terms.get(q).copied().unwrap_or(0.0);
                    idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len / avg.max(1.0)))
                })
                .sum::<f64>();
            (page, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored
}

/// The system-prompt section: the pages that matter for `query` in full, the rest by title.
pub fn prompt(pages: &[Page], query: &str) -> Option<String> {
    if pages.is_empty() {
        return Some(
            "\n\nKnowledge pages: none yet. When the user shares lasting, detailed context (a project, \
             a client, how they want something done), offer to keep it as a page with `knowledge_write`."
                .into(),
        );
    }
    let ranked = rank(pages, query);
    let best = ranked.first().map_or(0.0, |r| r.1);
    // Recall only clear matches: within reach of the best score, and above a floor.
    let recalled: Vec<&Page> = ranked.iter().filter(|(_, s)| *s >= 1.0 && *s >= best * 0.5).take(RECALL_PAGES).map(|(p, _)| *p).collect();
    let mut out = String::from(
        "\n\nKnowledge pages: the user's knowledge base. Pages relevant to this message are below; \
         read others with `knowledge_read`, search with `knowledge_search`, and create or update one \
         with `knowledge_write` when you learn something lasting. When an answer uses a page, cite it \
         inline as [Knowledge: Page title].",
    );
    for page in &recalled {
        let body: String = page.body.chars().take(RECALL_CHARS).collect();
        out.push_str(&format!("\n\n<knowledge_page id=\"{}\" title=\"{}\">\n{}\n</knowledge_page>", &page.id[..8], page.title, body));
    }
    let others: Vec<String> = pages
        .iter()
        .filter(|p| !recalled.iter().any(|r| r.id == p.id))
        .take(60)
        .map(|p| format!("{} ({})", p.title, &p.id[..8]))
        .collect();
    if !others.is_empty() {
        out.push_str(&format!("\nOther pages, by title: {}.", others.join("; ")));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_written_found_and_recalled_by_relevance() {
        let dir = tempfile::tempdir().unwrap();
        let acme = write(dir.path(), None, "Acme account", "Acme is our biggest client. Renewal is in March; contact is Rita.", "assistant", &["Chat: Q1 planning".into()]).unwrap();
        write(dir.path(), None, "Report style", "Reports: short, bullet points, numbers first.", "user", &[]).unwrap();
        write(dir.path(), None, "Garden", "Tomatoes go in the south bed in April.", "user", &[]).unwrap();

        // Updating by title keeps the page and merges its sources.
        let updated = write(dir.path(), None, "acme account", "Acme renewal moved to April.", "assistant", &["Mail from Rita".into()]).unwrap();
        assert_eq!(updated.id, acme.id);
        assert_eq!(updated.sources, ["Chat: Q1 planning", "Mail from Rita"]);
        let pages = load(dir.path());
        assert_eq!(pages.len(), 3);
        assert_eq!(find(&pages, &acme.id[..8]).unwrap().title, "acme account");

        let text = prompt(&pages, "When is the Acme renewal?").unwrap();
        assert!(text.contains("<knowledge_page") && text.contains("renewal moved to April"));
        assert!(!text.contains("Tomatoes"), "unrelated pages are listed, not recalled");
        assert!(text.contains("Garden ("));
        assert!(rank(&pages, "zzz unrelated").is_empty());

        let pages = delete(dir.path(), &acme.id).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(write(dir.path(), None, "  ", "x", "user", &[]).is_err());
    }
}
