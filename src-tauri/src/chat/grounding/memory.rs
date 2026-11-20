//! Knowledge: facts about the user the assistant keeps across conversations.
//!
//! Saved in `<config>/chat/memories.json`. Every memory goes into the system prompt while memory
//! is on, so recall needs no tool; the model adds and removes entries with `remember` and `forget`,
//! and the user edits them on the Knowledge page.

use crate::chat::error::ChatError;
use crate::support::settings_file::SettingsFile;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
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
    /// Where it applies: `project:<id>` or `agent:<id>`; everywhere when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
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
    /// `project:<id>` or `agent:<id>`, or everywhere.
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct Memories {
    /// Use what is known (memories and knowledge pages) in conversations.
    pub enabled: bool,
    /// Let the assistant save and change memories and knowledge pages.
    pub update: bool,
    pub items: Vec<Memory>,
    /// Vibe's "Sensitive data" switch: when on, the assistant does not save card numbers, bank
    /// accounts, passwords or keys to knowledge.
    pub keep_sensitive_out: bool,
}

impl Default for Memories {
    fn default() -> Self {
        Self {
            enabled: true,
            update: true,
            items: Vec::new(),
            keep_sensitive_out: true,
        }
    }
}

/// Why the assistant may not save `text`, when the user keeps sensitive data out.
pub(in crate::chat) fn refuse_sensitive(memories: &Memories, text: &str) -> Option<String> {
    let kinds = crate::chat::privacy::sensitive_kinds(text);
    (memories.keep_sensitive_out && !kinds.is_empty()).then(|| {
        format!("Not saved: it holds {}, and the user keeps sensitive data out of knowledge. Tell them you did not save it.", kinds.join(" and "))
    })
}

/// More than this and the oldest assistant-made memories make room.
pub const MAX_MEMORIES: usize = 200;
/// A memory is a fact, not a document.
pub const MAX_LENGTH: usize = 500;

const MEMORIES_FILE: SettingsFile<Memories> = SettingsFile::sealed("chat/memories.json");

pub(crate) fn load(config_dir: &Path) -> Memories {
    MEMORIES_FILE.load(config_dir)
}

pub(crate) fn save(config_dir: &Path, memories: &Memories) -> Result<(), ChatError> {
    // The memories as they were stay restorable.
    crate::chat::versions::keep(config_dir, "memories", "all", &load(config_dir), memories)?;
    MEMORIES_FILE
        .save(config_dir, memories)
        .map_err(String::from)
        .map_err(ChatError::Invalid)
}

/// Adds a memory, or returns the existing one when the same fact is already known.
pub fn add(memories: &mut Memories, text: &str, source: &str) -> Result<Memory, ChatError> {
    add_filed(memories, text, source, Filing::default())
}

/// Adds a memory with its category, importance and confidence.
pub(in crate::chat) fn add_filed(
    memories: &mut Memories,
    text: &str,
    source: &str,
    filing: Filing,
) -> Result<Memory, ChatError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ChatError::Invalid("A memory needs some text.".into()));
    }
    if text.chars().count() > MAX_LENGTH {
        return Err(ChatError::Invalid(format!(
            "A memory is at most {MAX_LENGTH} characters; save the key fact only."
        )));
    }
    if let Some(existing) = memories
        .items
        .iter()
        .find(|m| m.text.eq_ignore_ascii_case(text))
    {
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
        category: if CATEGORIES.contains(&category.as_str()) {
            category
        } else {
            String::new()
        },
        importance: filing.importance.unwrap_or(2).clamp(1, 3),
        confidence: if filing.inferred {
            "inferred".into()
        } else {
            "stated".into()
        },
        conversation: filing.conversation,
        scope: filing.scope,
    };
    memories.items.push(memory.clone());
    Ok(memory)
}

pub(crate) fn remove(memories: &mut Memories, id: &str) -> bool {
    let before = memories.items.len();
    memories.items.retain(|m| m.id != id);
    memories.items.len() != before
}

/// The part of the system prompt that carries the memories.
/// Past this many facts, only the important ones and those that fit the message are listed.
pub const LIST_ALL: usize = 40;
const LIST_RELEVANT: usize = 25;

/// Whether a fact applies in a chat of `project` with `agent`.
pub(in crate::chat::grounding) fn applies(
    m: &Memory,
    project: Option<&str>,
    agent: Option<&str>,
) -> bool {
    match m.scope.as_deref().and_then(|s| s.split_once(':')) {
        Some(("project", id)) => project == Some(id),
        Some(("agent", id)) => agent == Some(id),
        _ => true,
    }
}

/// The facts that apply here that best fit `query`, best first.
pub(crate) fn search<'a>(
    memories: &'a Memories,
    query: &str,
    project: Option<&str>,
    agent: Option<&str>,
) -> Vec<&'a Memory> {
    let here: Vec<&Memory> = memories
        .items
        .iter()
        .filter(|m| applies(m, project, agent))
        .collect();
    let corpus = crate::chat::retrieval::Corpus::new(
        here.iter()
            .map(|m| crate::chat::retrieval::tokens(&format!("{} {}", m.category, m.text))),
    )
    .with_prefixes();
    corpus
        .search(query)
        .into_iter()
        .map(|(i, _)| here[i])
        .collect()
}

/// The memory section of the system prompt for a chat of `project` with `agent`. With many facts,
/// a short index: the important ones and those that fit `query`, and how to find the rest.
pub(in crate::chat) fn prompt_for(
    memories: &Memories,
    query: &str,
    project: Option<&str>,
    agent: Option<&str>,
) -> Option<String> {
    if !memories.enabled {
        return None;
    }
    let shown = Memories {
        items: shown(memories, query, project, agent),
        ..memories.clone()
    };
    let mut out = render(&shown)?;
    let more = memories
        .items
        .iter()
        .filter(|m| applies(m, project, agent))
        .count()
        .saturating_sub(shown.items.len());
    if more > 0 {
        out.push_str(&format!(
            "\n({more} more facts are saved but not listed; find them with `memory_search`.)"
        ));
    }
    Some(out)
}

/// The facts [`prompt_for`] lists: all that apply when they are few, otherwise the important ones
/// and those that fit `query`. What a run is attributed as having had in front of it.
pub(crate) fn shown(
    memories: &Memories,
    query: &str,
    project: Option<&str>,
    agent: Option<&str>,
) -> Vec<Memory> {
    if !memories.enabled {
        return Vec::new();
    }
    let applicable: Vec<&Memory> = memories
        .items
        .iter()
        .filter(|m| applies(m, project, agent))
        .collect();
    let hidden = applicable.len().saturating_sub(LIST_ALL);
    let listed: Vec<Memory> = if hidden == 0 {
        applicable.into_iter().cloned().collect()
    } else {
        let mut keep: Vec<&Memory> = applicable
            .iter()
            .copied()
            .filter(|m| m.importance == 3)
            .collect();
        for m in search(memories, query, project, agent) {
            if keep.len() >= LIST_RELEVANT {
                break;
            }
            if !keep.iter().any(|k| k.id == m.id) {
                keep.push(m);
            }
        }
        keep.into_iter().cloned().collect()
    };
    listed
}

fn render(memories: &Memories) -> Option<String> {
    if !memories.enabled {
        return None;
    }
    let mut out = String::from(if memories.update {
        "\n\nMemory: you can save lasting facts about the user with `remember` (preferences, \
         their work, people and projects they mention, how they like answers) and remove wrong \
         ones with `forget`. Save only what will help in future conversations, never secrets. \
         When the user asks you to remember something, call `remember` once per fact before you \
         answer, and say it is saved only once the call has succeeded: nothing is kept otherwise. \
         Use what you know naturally; do not recite it."
    } else {
        "\n\nMemory: what you know about the user is below. The user has turned off saving new \
         facts, so do not offer to remember anything. Use what you know naturally; do not recite it."
    });
    if memories.items.is_empty() {
        out.push_str(" You know nothing about the user yet.");
    } else {
        out.push_str("\nWhat you know about the user (id: fact), by subject, the most important first; \
            facts marked (inferred) are your own conclusions, so check them before relying on them:");
        let mut groups: Vec<&str> = CATEGORIES.to_vec();
        groups.push("");
        for group in groups {
            let mut items: Vec<&Memory> = memories
                .items
                .iter()
                .filter(|m| {
                    m.category == group
                        || (group.is_empty() && !CATEGORIES.contains(&m.category.as_str()))
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            items.sort_by_key(|m| std::cmp::Reverse(m.importance));
            out.push_str(&format!(
                "\n{}:",
                if group.is_empty() { "Other" } else { group }
            ));
            for m in items {
                let mark = match (m.importance, m.confidence.as_str()) {
                    (3, "inferred") => " (important, inferred)",
                    (3, _) => " (important)",
                    (_, "inferred") => " (inferred)",
                    _ => "",
                };
                out.push_str(&format!(
                    "\n- {}: {}{mark}",
                    &m.id[..8.min(m.id.len())],
                    m.text
                ));
            }
        }
    }
    Some(out)
}

/// The kernel's `consolidate(scope)` for facts: duplicates within a scope (`None` for all) are
/// merged into the one kept, which keeps the highest importance. Returns how many were merged.
/// The memories as they were stay restorable (`save` keeps a version).
pub(in crate::chat) fn consolidate(
    config_dir: &std::path::Path,
    scope: Option<&str>,
) -> Result<usize, ChatError> {
    let mut memories = load(config_dir);
    let facts: Vec<cortex_kernel::lifecycle::Fact> = memories
        .items
        .iter()
        .filter(|m| scope.is_none() || m.scope.as_deref() == scope)
        .map(|m| cortex_kernel::lifecycle::Fact {
            id: m.id.clone(),
            text: m.text.clone(),
            importance: m.importance,
            scope: m.scope.clone(),
        })
        .collect();
    let groups = cortex_kernel::lifecycle::duplicates(&facts);
    // Like the rest of the lifecycle, it removes only what the assistant wrote: a fact the user
    // wrote can be the one kept, never one dropped.
    let by_user: std::collections::HashSet<&String> = memories
        .items
        .iter()
        .filter(|m| m.source == "user")
        .map(|m| &m.id)
        .collect();
    let gone: std::collections::HashSet<&String> = groups
        .iter()
        .flat_map(|(_, d)| d)
        .filter(|id| !by_user.contains(id))
        .collect();
    if gone.is_empty() {
        return Ok(0);
    }
    memories.items.retain(|m| !gone.contains(&m.id));
    save(config_dir, &memories)?;
    Ok(gone.len())
}

/// Finds a memory by its full id or the short id shown in the prompt.
pub(crate) fn resolve<'a>(memories: &'a Memories, id: &str) -> Option<&'a Memory> {
    let id = id.trim();
    memories.items.iter().find(|m| m.id == id).or_else(|| {
        (id.len() >= 6)
            .then(|| memories.items.iter().find(|m| m.id.starts_with(id)))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every change to the memories keeps the version it replaces, so any can be undone.
    #[test]
    fn changing_memories_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut memories = load(dir.path());
        add(&mut memories, "Prefers metric units", "user").unwrap();
        save(dir.path(), &memories).unwrap();
        add(&mut memories, "Lives in Lisbon", "assistant").unwrap();
        save(dir.path(), &memories).unwrap();
        let versions = crate::chat::versions::list(dir.path(), "memories", "all").unwrap();
        assert_eq!(versions.len(), 2);
        let previous = serde_json::to_string(&versions[0].value).unwrap();
        assert!(
            previous.contains("metric") && !previous.contains("Lisbon"),
            "{previous}"
        );
    }

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
        let text = prompt_for(&m, "", None, None).unwrap();
        assert!(text.contains("Works at a solar company"));
        assert!(text.contains(&fact.id[..8]));
        assert_eq!(resolve(&m, &fact.id[..8]).unwrap().id, fact.id);
        m.enabled = false;
        assert!(prompt_for(&m, "", None, None).is_none());
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
        add_filed(
            &mut m,
            "Prefers metric units",
            "assistant",
            Filing {
                category: "Preference".into(),
                importance: Some(3),
                inferred: false,
                conversation: Some("c1".into()),
                scope: None,
            },
        )
        .unwrap();
        add_filed(
            &mut m,
            "Probably works in fintech",
            "assistant",
            Filing {
                category: "work".into(),
                importance: Some(1),
                inferred: true,
                conversation: None,
                scope: None,
            },
        )
        .unwrap();
        add(&mut m, "Lives in Porto", "user").unwrap();
        add_filed(
            &mut m,
            "Odd one",
            "assistant",
            Filing {
                category: "astrology".into(),
                importance: Some(9),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(m.items[0].category, "preference");
        assert_eq!(m.items[0].conversation.as_deref(), Some("c1"));
        assert_eq!(m.items[1].confidence, "inferred");
        assert_eq!(
            (
                m.items[2].category.as_str(),
                m.items[2].importance,
                m.items[2].confidence.as_str()
            ),
            ("", 2, "stated")
        );
        assert_eq!(
            (m.items[3].category.as_str(), m.items[3].importance),
            ("", 3)
        );
        let p = prompt_for(&m, "", None, None).unwrap();
        assert!(p.contains("preference:\n- ") && p.contains("Prefers metric units (important)"));
        assert!(p.contains("Probably works in fintech (inferred)"));
        assert!(p.find("preference:").unwrap() < p.find("work:").unwrap());
        assert!(p.contains("Other:"));
        // Old saved memories, without the new fields, still load.
        let old: Memory =
            serde_json::from_str(r#"{"id":"x","text":"t","createdAt":"2026-01-01T00:00:00Z"}"#)
                .unwrap();
        assert_eq!((old.importance, old.confidence.as_str()), (2, "stated"));
        // When full, the least important assistant fact goes first.
        let mut full = Memories::default();
        for i in 0..MAX_MEMORIES {
            add_filed(
                &mut full,
                &format!("fact {i}"),
                "assistant",
                Filing {
                    importance: Some(if i == 50 { 1 } else { 2 }),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        add(&mut full, "new", "user").unwrap();
        assert!(!full.items.iter().any(|x| x.text == "fact 50"));
        assert!(full.items.iter().any(|x| x.text == "fact 0"));
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;

    fn fact(memories: &mut Memories, text: &str, importance: u8, scope: Option<&str>) {
        add_filed(
            memories,
            text,
            "user",
            Filing {
                importance: Some(importance),
                scope: scope.map(str::to_string),
                ..Default::default()
            },
        )
        .unwrap();
    }

    /// Many facts become a short index: the important ones and those that fit the message.
    #[test]
    fn many_facts_become_a_short_index_and_scopes_apply() {
        let mut m = Memories::default();
        for i in 0..60 {
            fact(
                &mut m,
                &format!("Detail number {i} about gardening"),
                2,
                None,
            );
        }
        fact(&mut m, "Allergic to penicillin", 3, None);
        fact(&mut m, "Prefers invoices in euros", 2, None);
        fact(
            &mut m,
            "The Atlas project ships in May",
            2,
            Some("project:atlas"),
        );
        let prompt = prompt_for(&m, "send the invoices", None, None).unwrap();
        assert!(prompt.contains("penicillin"), "important facts always show");
        assert!(prompt.contains("invoices in euros"), "relevant facts show");
        assert!(prompt.contains("more facts are saved"));
        assert!(
            !prompt.contains("Atlas"),
            "another project's fact stays out"
        );
        assert!(prompt.matches("\n- ").count() <= LIST_RELEVANT);
        let in_atlas = search(&m, "Atlas ships", Some("atlas"), None);
        assert_eq!(in_atlas[0].text, "The Atlas project ships in May");
        assert!(search(&m, "Atlas ships", Some("other"), None)
            .iter()
            .all(|f| !f.text.contains("Atlas")));
        // A few facts are listed in full, as before.
        let mut few = Memories::default();
        fact(&mut few, "Likes tea", 2, None);
        assert!(prompt_for(&few, "", None, None)
            .unwrap()
            .contains("Likes tea"));
    }
}

#[cfg(test)]
mod consolidate_tests {
    use super::*;

    #[test]
    fn duplicate_facts_are_merged_and_the_old_list_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut m = load(d);
        for (text, importance) in [
            ("Prefers metric units", 2),
            ("prefers metric units.", 3),
            ("Lives in Lisbon", 2),
        ] {
            add_filed(
                &mut m,
                text,
                "assistant",
                Filing {
                    importance: Some(importance),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        // add_filed already skips exact duplicates; these differ in case and punctuation.
        save(d, &m).unwrap();
        assert_eq!(load(d).items.len(), 3);
        // The user's own duplicates stay: the lifecycle never removes what the user wrote.
        let mut with_user = load(d);
        add(&mut with_user, "Works at Acme", "user").unwrap();
        add(&mut with_user, "works at Acme.", "user").unwrap();
        save(d, &with_user).unwrap();
        assert_eq!(consolidate(d, None).unwrap(), 1);
        let kept = load(d);
        assert_eq!(kept.items.len(), 4);
        assert_eq!(kept.items.iter().filter(|f| f.source == "user").count(), 2);
        assert!(kept
            .items
            .iter()
            .any(|f| f.importance == 3 && f.text.contains("metric")));
        // Undoable: the list before consolidation is a version.
        let versions = crate::chat::versions::list(d, "memories", "all").unwrap();
        assert!(serde_json::to_string(&versions[0].value)
            .unwrap()
            .contains("Prefers metric units"));
        assert_eq!(consolidate(d, None).unwrap(), 0);
    }
}
