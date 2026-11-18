//! Knowledge: facts about the user the assistant keeps across conversations.
//!
//! Saved in `<config>/chat/memories.json`. Every memory goes into the system prompt while memory
//! is on, so recall needs no tool; the model adds and removes entries with `remember` and `forget`,
//! and the user edits them on the Knowledge page.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::store::write_json;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    /// `user` when typed on the Knowledge page, `assistant` when the model saved it.
    #[serde(default)]
    pub source: String,
    /// What the fact is about: one of [`CATEGORIES`], or empty.
    #[serde(default)]
    pub category: String,
    /// How much it matters, 1 (a detail) to 3 (always keep in mind); 2 when not said.
    #[serde(default = "normal")]
    pub importance: u8,
    /// The user said it (`stated`), or the assistant concluded it (`inferred`).
    #[serde(default = "stated")]
    pub confidence: String,
    /// The conversation it was learned in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

pub const CATEGORIES: &[&str] = &["preference", "work", "person", "project", "habit", "fact"];

fn normal() -> u8 {
    2
}

fn stated() -> String {
    "stated".into()
}

/// How a fact is filed: its category, importance and confidence, checked.
#[derive(Debug, Clone, Default)]
pub struct Filing {
    pub category: String,
    pub importance: Option<u8>,
    pub inferred: bool,
    pub conversation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Memories {
    pub enabled: bool,
    pub items: Vec<Memory>,
    /// Vibe's "Sensitive data" switch: when on, the assistant does not save card numbers, bank
    /// accounts, passwords or keys to knowledge.
    pub keep_sensitive_out: bool,
}

impl Default for Memories {
    fn default() -> Self {
        Self { enabled: true, items: Vec::new(), keep_sensitive_out: true }
    }
}

/// Why the assistant may not save `text`, when the user keeps sensitive data out.
pub fn refuse_sensitive(memories: &Memories, text: &str) -> Option<String> {
    let kinds = super::privacy::sensitive_kinds(text);
    (memories.keep_sensitive_out && !kinds.is_empty()).then(|| {
        format!("Not saved: it holds {}, and the user keeps sensitive data out of knowledge. Tell them you did not save it.", kinds.join(" and "))
    })
}

/// More than this and the oldest assistant-made memories make room.
pub const MAX_MEMORIES: usize = 200;
/// A memory is a fact, not a document.
pub const MAX_LENGTH: usize = 500;

fn path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("memories.json")
}

pub fn load(config_dir: &Path) -> Memories {
    std::fs::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, memories: &Memories) -> Result<(), String> {
    write_json(&path(config_dir), memories)
}

/// Adds a memory, or returns the existing one when the same fact is already known.
pub fn add(memories: &mut Memories, text: &str, source: &str) -> Result<Memory, String> {
    add_filed(memories, text, source, Filing::default())
}

/// Adds a memory with its category, importance and confidence.
pub fn add_filed(memories: &mut Memories, text: &str, source: &str, filing: Filing) -> Result<Memory, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A memory needs some text.".into());
    }
    if text.chars().count() > MAX_LENGTH {
        return Err(format!("A memory is at most {MAX_LENGTH} characters; save the key fact only."));
    }
    if let Some(existing) = memories.items.iter().find(|m| m.text.eq_ignore_ascii_case(text)) {
        return Ok(existing.clone());
    }
    // When full, the least important of the assistant's own facts goes first, oldest first.
    while memories.items.len() >= MAX_MEMORIES {
        let drop = memories
            .items
            .iter()
            .enumerate()
            .filter(|(_, m)| m.source == "assistant")
            .min_by_key(|(i, m)| (m.importance, *i))
            .map_or(0, |(i, _)| i);
        memories.items.remove(drop);
    }
    let category = filing.category.trim().to_lowercase();
    let memory = Memory {
        id: uuid::Uuid::new_v4().to_string(),
        text: text.to_string(),
        created_at: Utc::now(),
        source: source.to_string(),
        category: if CATEGORIES.contains(&category.as_str()) { category } else { String::new() },
        importance: filing.importance.unwrap_or(2).clamp(1, 3),
        confidence: if filing.inferred { "inferred".into() } else { "stated".into() },
        conversation: filing.conversation,
    };
    memories.items.push(memory.clone());
    Ok(memory)
}

pub fn remove(memories: &mut Memories, id: &str) -> bool {
    let before = memories.items.len();
    memories.items.retain(|m| m.id != id);
    memories.items.len() != before
}

/// The part of the system prompt that carries the memories.
pub fn prompt(memories: &Memories) -> Option<String> {
    if !memories.enabled {
        return None;
    }
    let mut out = String::from(
        "\n\nMemory: you can save lasting facts about the user with `remember` (preferences, \
         their work, people and projects they mention, how they like answers) and remove wrong \
         ones with `forget`. Save only what will help in future conversations, never secrets. \
         Use what you know naturally; do not recite it.",
    );
    if memories.items.is_empty() {
        out.push_str(" You know nothing about the user yet.");
    } else {
        out.push_str("\nWhat you know about the user (id: fact), by subject, the most important first; \
            facts marked (inferred) are your own conclusions, so check them before relying on them:");
        let mut groups: Vec<&str> = CATEGORIES.to_vec();
        groups.push("");
        for group in groups {
            let mut items: Vec<&Memory> = memories.items.iter().filter(|m| m.category == group || (group.is_empty() && !CATEGORIES.contains(&m.category.as_str()))).collect();
            if items.is_empty() {
                continue;
            }
            items.sort_by_key(|m| std::cmp::Reverse(m.importance));
            out.push_str(&format!("\n{}:", if group.is_empty() { "Other" } else { group }));
            for m in items {
                let mark = match (m.importance, m.confidence.as_str()) {
                    (3, "inferred") => " (important, inferred)",
                    (3, _) => " (important)",
                    (_, "inferred") => " (inferred)",
                    _ => "",
                };
                out.push_str(&format!("\n- {}: {}{mark}", &m.id[..8.min(m.id.len())], m.text));
            }
        }
    }
    Some(out)
}

/// Finds a memory by its full id or the short id shown in the prompt.
pub fn resolve<'a>(memories: &'a Memories, id: &str) -> Option<&'a Memory> {
    let id = id.trim();
    memories
        .items
        .iter()
        .find(|m| m.id == id)
        .or_else(|| (id.len() >= 6).then(|| memories.items.iter().find(|m| m.id.starts_with(id))).flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memories_are_deduplicated_and_bounded() {
        let mut m = Memories::default();
        let a = add(&mut m, "Prefers metric units", "assistant").unwrap();
        let again = add(&mut m, "prefers metric units", "assistant").unwrap();
        assert_eq!(a.id, again.id);
        assert!(add(&mut m, "  ", "user").is_err());
        assert!(add(&mut m, &"x".repeat(MAX_LENGTH + 1), "user").is_err());
        for i in 0..MAX_MEMORIES + 5 {
            add(&mut m, &format!("fact {i}"), "assistant").unwrap();
        }
        assert_eq!(m.items.len(), MAX_MEMORIES);
    }

    #[test]
    fn the_prompt_lists_memories_by_short_id() {
        let mut m = Memories::default();
        let fact = add(&mut m, "Works at a solar company", "user").unwrap();
        let text = prompt(&m).unwrap();
        assert!(text.contains("Works at a solar company"));
        assert!(text.contains(&fact.id[..8]));
        assert_eq!(resolve(&m, &fact.id[..8]).unwrap().id, fact.id);
        m.enabled = false;
        assert!(prompt(&m).is_none());
    }

    #[test]
    fn removal_reports_whether_anything_went() {
        let mut m = Memories::default();
        let fact = add(&mut m, "Has two cats", "user").unwrap();
        assert!(remove(&mut m, &fact.id));
        assert!(!remove(&mut m, &fact.id));
    }

    #[test]
    fn facts_are_filed_and_weighed() {
        let mut m = Memories::default();
        add_filed(&mut m, "Prefers metric units", "assistant", Filing { category: "Preference".into(), importance: Some(3), inferred: false, conversation: Some("c1".into()) }).unwrap();
        add_filed(&mut m, "Probably works in fintech", "assistant", Filing { category: "work".into(), importance: Some(1), inferred: true, conversation: None }).unwrap();
        add(&mut m, "Lives in Porto", "user").unwrap();
        add_filed(&mut m, "Odd one", "assistant", Filing { category: "astrology".into(), importance: Some(9), ..Default::default() }).unwrap();
        assert_eq!(m.items[0].category, "preference");
        assert_eq!(m.items[0].conversation.as_deref(), Some("c1"));
        assert_eq!(m.items[1].confidence, "inferred");
        assert_eq!((m.items[2].category.as_str(), m.items[2].importance, m.items[2].confidence.as_str()), ("", 2, "stated"));
        assert_eq!((m.items[3].category.as_str(), m.items[3].importance), ("", 3));
        let p = prompt(&m).unwrap();
        assert!(p.contains("preference:\n- ") && p.contains("Prefers metric units (important)"));
        assert!(p.contains("Probably works in fintech (inferred)"));
        assert!(p.find("preference:").unwrap() < p.find("work:").unwrap());
        assert!(p.contains("Other:"));
        // Old saved memories, without the new fields, still load.
        let old: Memory = serde_json::from_str(r#"{"id":"x","text":"t","createdAt":"2026-01-01T00:00:00Z"}"#).unwrap();
        assert_eq!((old.importance, old.confidence.as_str()), (2, "stated"));
        // When full, the least important assistant fact goes first.
        let mut full = Memories::default();
        for i in 0..MAX_MEMORIES {
            add_filed(&mut full, &format!("fact {i}"), "assistant", Filing { importance: Some(if i == 50 { 1 } else { 2 }), ..Default::default() }).unwrap();
        }
        add(&mut full, "new", "user").unwrap();
        assert!(!full.items.iter().any(|x| x.text == "fact 50"));
        assert!(full.items.iter().any(|x| x.text == "fact 0"));
    }

}
