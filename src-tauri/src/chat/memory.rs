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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Memories {
    pub enabled: bool,
    pub items: Vec<Memory>,
}

impl Default for Memories {
    fn default() -> Self {
        Self { enabled: true, items: Vec::new() }
    }
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
    while memories.items.len() >= MAX_MEMORIES {
        let oldest = memories
            .items
            .iter()
            .position(|m| m.source == "assistant")
            .unwrap_or(0);
        memories.items.remove(oldest);
    }
    let memory = Memory {
        id: uuid::Uuid::new_v4().to_string(),
        text: text.to_string(),
        created_at: Utc::now(),
        source: source.to_string(),
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
        out.push_str("\nWhat you know about the user (id: fact):");
        for m in &memories.items {
            out.push_str(&format!("\n- {}: {}", &m.id[..8.min(m.id.len())], m.text));
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
}
