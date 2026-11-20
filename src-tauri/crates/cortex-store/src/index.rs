//! The conversation index: what makes the sidebar and search stay fast as history grows.
//!
//! Conversation files stay the source of truth. This is derived from them and rebuilt whenever it
//! is missing or out of date: `chat/index/conversations.json` holds each conversation's sidebar
//! row with the file's size and modification time, and `chat/index/text/<id>.txt` its searchable
//! text. Listing compares sizes and times and parses only the files that changed, so a history of
//! thousands of conversations costs one `stat` each instead of a full parse. A file that cannot
//! be read is recorded as damaged, not silently skipped.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::{chat_dir, write_json, Conversation, ConversationSummary};

/// Bumped when the index's own shape changes; an older index is rebuilt.
const INDEX_VERSION: u32 = 6;
/// Characters of text kept per conversation for search.
const MAX_TEXT: usize = 400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    modified: u128,
    size: u64,
    /// `None` for a temporary conversation or a damaged file.
    summary: Option<ConversationSummary>,
    #[serde(default)]
    damaged: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    version: u32,
    /// By conversation id (the file's stem).
    entries: BTreeMap<String, Entry>,
}

fn index_dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("index")
}

fn text_path(config_dir: &Path, id: &str) -> PathBuf {
    index_dir(config_dir).join("text").join(format!("{id}.txt"))
}

fn cache() -> std::sync::MutexGuard<'static, HashMap<PathBuf, IndexFile>> {
    // Process-wide: the index on disk is the one list of conversations; a second copy in memory
    // (one per caller) would show a list another caller had already changed.
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, IndexFile>>> = OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn summarize(c: &Conversation) -> ConversationSummary {
    ConversationSummary {
        archived: c.archived,
        tags: c.tags.clone(),
        title: if c.title.is_empty() {
            "New chat".into()
        } else {
            c.title.clone()
        },
        id: c.id.clone(),
        project_id: c.project_id.clone(),
        workdir: c.workdir.clone(),
        agent_id: c.agent_id.clone(),
        task_id: c.task_id.clone(),
        unread: c.unread,
        pinned: c.pinned,
        created_at: c.created_at,
        updated_at: c.updated_at,
        tokens: c.usage.input_tokens + c.usage.output_tokens,
        cost: c.usage.cost,
        model: c.model.clone(),
        preview: preview(c),
        bookmarks: c.bookmarks.clone(),
    }
}

/// The last thing said with text in it, on one line, short.
fn preview(c: &Conversation) -> String {
    let text = c
        .messages
        .iter()
        .rev()
        .map(cortex_kernel::context::message_text)
        .find(|t| !t.trim().is_empty() && !t.starts_with('{'))
        .unwrap_or_default();
    // Markdown marks read as noise in a tooltip.
    let plain: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '#' | '`' | '>' | '_'))
        .collect();
    let flat: String = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 160 {
        format!("{}…", crate::text::clip(&flat, 160))
    } else {
        flat
    }
}

/// What search reads: the title, then what was said, one message a line.
fn searchable(c: &Conversation) -> String {
    let mut text = format!("{}\n", c.title);
    for message in &c.messages {
        let said = cortex_kernel::context::message_text(message);
        if !said.trim().is_empty() {
            text.push_str(said.trim());
            text.push('\n');
        }
        if text.len() > MAX_TEXT {
            break;
        }
    }
    if text.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// Brings the index in line with the conversation files and returns it.
fn synced<R>(config_dir: &Path, read: impl FnOnce(&IndexFile) -> R) -> R {
    let mut cache = cache();
    let index = cache.entry(config_dir.to_path_buf()).or_insert_with(|| {
        crate::at_rest::read_to_string(index_dir(config_dir).join("conversations.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<IndexFile>(&t).ok())
            .filter(|i| i.version == INDEX_VERSION)
            .unwrap_or_default()
    });
    index.version = INDEX_VERSION;
    let mut dirty = false;
    let mut seen: Vec<String> = Vec::new();
    let files = std::fs::read_dir(chat_dir(config_dir).join("conversations"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok);
    for file in files {
        let path = file.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(meta) = file.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis());
        seen.push(id.clone());
        // A damaged entry is looked at again: the cause may have passed (a locked keychain).
        if index
            .entries
            .get(&id)
            .is_some_and(|e| !e.damaged && e.modified == modified && e.size == meta.len())
        {
            continue;
        }
        dirty = true;
        let parsed = crate::at_rest::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Conversation>(&t).ok());
        let text = text_path(config_dir, &id);
        let entry = match parsed {
            Some(c) if c.temporary => {
                let _ = std::fs::remove_file(&text);
                Entry {
                    modified,
                    size: meta.len(),
                    summary: None,
                    damaged: false,
                }
            }
            Some(c) => {
                if let Some(parent) = text.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = crate::at_rest::write(&text, searchable(&c));
                Entry {
                    modified,
                    size: meta.len(),
                    summary: Some(summarize(&c)),
                    damaged: false,
                }
            }
            None => {
                log::warn!(
                    "conversation file {path:?} cannot be read; kept and reported as damaged"
                );
                Entry {
                    modified,
                    size: meta.len(),
                    summary: None,
                    damaged: true,
                }
            }
        };
        index.entries.insert(id, entry);
    }
    if index.entries.len() != seen.len() {
        let gone: Vec<String> = index
            .entries
            .keys()
            .filter(|id| !seen.contains(id))
            .cloned()
            .collect();
        for id in gone {
            index.entries.remove(&id);
            let _ = std::fs::remove_file(text_path(config_dir, &id));
            dirty = true;
        }
    }
    if dirty {
        if let Err(e) = write_json(&index_dir(config_dir).join("conversations.json"), index) {
            log::warn!("could not save the conversation index: {e}");
        }
    }
    read(index)
}

/// The sidebar rows: pinned first, then newest.
pub fn list(config_dir: &Path) -> Vec<ConversationSummary> {
    let mut out: Vec<ConversationSummary> = synced(config_dir, |index| {
        index
            .entries
            .values()
            .filter_map(|e| e.summary.clone())
            .collect()
    });
    out.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_at.cmp(&a.updated_at))
    });
    out
}

/// Conversation files that exist but cannot be read.
pub fn damaged(config_dir: &Path) -> Vec<String> {
    synced(config_dir, |index| {
        index
            .entries
            .iter()
            .filter(|(_, e)| e.damaged)
            .map(|(id, _)| id.clone())
            .collect()
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    feature = "typescript",
    derive(ts_rs::TS),
    ts(export, rename = "SearchHit")
)]
pub struct Hit {
    pub id: String,
    pub title: String,
    /// The words around the first match in what was said; empty when only the title matches.
    pub snippet: String,
    pub in_title: bool,
    pub archived: bool,
    pub tags: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

fn snippet(text: &str, lower: &str, term: &str) -> String {
    let Some(at) = lower.find(term) else {
        return String::new();
    };
    // Lowercasing can change byte lengths; fall back to the lowercased text when it did.
    let source = if lower.len() == text.len() {
        text
    } else {
        lower
    };
    let (mut start, mut end) = (
        at.saturating_sub(60),
        (at + term.len() + 100).min(source.len()),
    );
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    while !source.is_char_boundary(end) {
        end += 1;
    }
    let body = source[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}{body}{}",
        if start > 0 { "…" } else { "" },
        if end < source.len() { "…" } else { "" }
    )
}

/// Full-text search over titles and what was said: every word must appear. Title matches come
/// first, then the newest.
pub fn search(config_dir: &Path, query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = list(config_dir)
        .into_iter()
        .filter_map(|c| {
            let title = c.title.to_lowercase();
            let in_title = terms.iter().all(|t| title.contains(t));
            let text =
                crate::at_rest::read_to_string(text_path(config_dir, &c.id)).unwrap_or_default();
            // The first line repeats the title; snippets come from what was said.
            let said = text.split_once('\n').map_or("", |(_, rest)| rest);
            let lower = said.to_lowercase();
            let in_text = terms.iter().all(|t| lower.contains(t) || title.contains(t))
                && terms.iter().any(|t| lower.contains(t));
            if !in_title && !in_text {
                return None;
            }
            let first = terms.iter().find(|t| lower.contains(t.as_str()));
            Some(Hit {
                snippet: first.map_or(String::new(), |t| snippet(said, &lower, t)),
                id: c.id,
                title: c.title,
                in_title,
                archived: c.archived,
                tags: c.tags,
                updated_at: c.updated_at,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.in_title
            .cmp(&a.in_title)
            .then(b.updated_at.cmp(&a.updated_at))
    });
    hits.truncate(limit);
    hits
}

/// Earlier conversations matching `query`: every word first; failing that, the most words.
pub fn search_chats(config_dir: &Path, query: &str) -> Vec<Hit> {
    let all = search(config_dir, query, 8);
    let words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| w.chars().count() > 2)
        .collect();
    if !all.is_empty() || words.len() < 2 {
        return all;
    }
    let mut found: Vec<(usize, crate::index::Hit)> = Vec::new();
    for word in words {
        for hit in search(config_dir, word, 20) {
            match found.iter_mut().find(|(_, h)| h.id == hit.id) {
                Some((n, _)) => *n += 1,
                None => found.push((1, hit)),
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.updated_at.cmp(&a.1.updated_at)));
    found.into_iter().take(8).map(|(_, h)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{delete_conversation, save_conversation};

    fn conversation(id: &str, title: &str, said: &str) -> Conversation {
        let mut c = Conversation::new(id.into(), None, None);
        c.title = title.into();
        c.messages.push(rig_core::completion::Message::user(said));
        c
    }

    #[test]
    fn lists_from_the_index_and_follows_changes() {
        let dir = tempfile::tempdir().unwrap();
        save_conversation(
            dir.path(),
            &conversation(
                "11111111-1111-4111-8111-111111111111",
                "Trip to Porto",
                "Book the Douro valley train",
            ),
        )
        .unwrap();
        let mut temp = conversation(
            "22222222-2222-4222-8222-222222222222",
            "Secret",
            "a hidden thing",
        );
        temp.temporary = true;
        save_conversation(dir.path(), &temp).unwrap();
        assert_eq!(list(dir.path()).len(), 1);
        assert!(index_dir(dir.path()).join("conversations.json").exists());

        // Search finds what was said, not just titles, and leaves temporary chats out.
        let hits = search(dir.path(), "douro TRAIN", 10);
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].in_title && hits[0].snippet.contains("Douro valley"));
        assert!(search(dir.path(), "porto", 10)[0].in_title);
        assert!(search(dir.path(), "hidden", 10).is_empty());
        assert!(search(dir.path(), "douro missing", 10).is_empty());

        // A rebuilt index gives the same answer.
        cache().remove(dir.path());
        std::fs::remove_dir_all(index_dir(dir.path())).unwrap();
        assert_eq!(search(dir.path(), "douro", 10).len(), 1);

        // A file that cannot be read is reported, and the rest still list.
        let bad =
            chat_dir(dir.path()).join("conversations/33333333-3333-4333-8333-333333333333.json");
        crate::at_rest::write(&bad, "{ not json").unwrap();
        assert_eq!(list(dir.path()).len(), 1);
        assert_eq!(
            damaged(dir.path()),
            vec!["33333333-3333-4333-8333-333333333333".to_string()]
        );

        delete_conversation(dir.path(), "11111111-1111-4111-8111-111111111111").unwrap();
        assert!(list(dir.path()).is_empty());
        assert!(search(dir.path(), "douro", 10).is_empty());
    }
}
