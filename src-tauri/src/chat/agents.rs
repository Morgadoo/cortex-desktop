//! Agents: named assistants with their own persona, model, tools and standing tasks.
//!
//! The workforce: each agent is a reusable configuration a conversation or a scheduled task runs
//! under. Its instructions replace the generic persona, its model overrides the composer's, and
//! its tool list narrows what it may use. Saved in `<config>/chat/agents.json`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// An emoji or one or two letters.
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub description: String,
    /// Who the agent is and how it works; replaces the generic assistant persona.
    #[serde(default)]
    pub instructions: String,
    /// `vendor/model`; the composer's model when empty.
    #[serde(default)]
    pub model: Option<String>,
    /// Tool names (`web_search`) or connector ids (`filesystem`) it may use; all when empty.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Skills it may load; all enabled ones when empty.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Think longer by default.
    #[serde(default)]
    pub think: bool,
    pub created_at: DateTime<Utc>,
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("agents.json")
}

pub fn load(config_dir: &Path) -> Vec<Agent> {
    std::fs::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn find(config_dir: &Path, id: &str) -> Option<Agent> {
    load(config_dir).into_iter().find(|a| a.id == id)
}

pub fn save(config_dir: &Path, agent: Agent) -> Result<Vec<Agent>, String> {
    if !valid_id(&agent.id) {
        return Err("invalid agent id".into());
    }
    if agent.name.trim().is_empty() {
        return Err("An agent needs a name.".into());
    }
    let mut agents = load(config_dir);
    match agents.iter_mut().find(|a| a.id == agent.id) {
        Some(existing) => *existing = agent,
        None => agents.push(agent),
    }
    write_json(&path(config_dir), &agents)?;
    Ok(agents)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Agent>, String> {
    let mut agents = load(config_dir);
    agents.retain(|a| a.id != id);
    write_json(&path(config_dir), &agents)?;
    Ok(agents)
}

/// Whether `agent` may use the tool `name` (`connector__tool` or a built-in name).
pub fn allows(agent: &Agent, name: &str) -> bool {
    if agent.tools.is_empty() {
        return true;
    }
    let connector = name.split_once("__").map_or(name, |(c, _)| c);
    agent.tools.iter().any(|t| t == name || t == connector)
}

/// Ready-made agents a user can add with one click.
pub fn templates() -> Vec<Agent> {
    let make = |id: &str, name: &str, avatar: &str, description: &str, instructions: &str, tools: &[&str]| Agent {
        id: id.into(),
        name: name.into(),
        avatar: avatar.into(),
        description: description.into(),
        instructions: instructions.into(),
        model: None,
        tools: tools.iter().map(|s| s.to_string()).collect(),
        skills: Vec::new(),
        think: false,
        created_at: Utc::now(),
    };
    vec![
        make(
            "researcher",
            "Researcher",
            "🔎",
            "Finds, verifies and summarizes information from the web with sources.",
            "You are a meticulous research analyst. Plan before searching, prefer primary and official sources, cross-check key figures, delegate broad questions to sub-agents, and always cite. Flag uncertainty plainly.",
            &["web_search", "browser", "delegate", "update_plan", "canvas", "use_skill"],
        ),
        make(
            "news-briefer",
            "News Briefer",
            "📰",
            "Writes a concise briefing of what changed in a topic since last time.",
            "You write short, skimmable briefings: the 5 most important developments, one or two sentences each, with the date and a source link. Lead with what changed and why it matters. No filler.",
            &["web_search", "browser", "delegate"],
        ),
        make(
            "file-organizer",
            "File Assistant",
            "🗂️",
            "Searches, reads and summarizes documents in the folders you share.",
            "You help the user with the documents in their shared folders: find files, read them, summarize and compare them, and propose clear next steps. Never change a file unless asked.",
            &["filesystem", "update_plan", "canvas"],
        ),
        make(
            "writer",
            "Writer",
            "✍️",
            "Drafts and edits documents in a canvas, matching your tone.",
            "You are a precise editor and writer. Ask for audience and goal only when they cannot be inferred. Write in a canvas, keep the chat reply to a line, and offer one alternative when useful.",
            &["canvas", "web_search", "use_skill"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_save_and_restrict_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = templates().remove(0);
        agent.id = "a1".into();
        save(dir.path(), agent.clone()).unwrap();
        assert_eq!(find(dir.path(), "a1").unwrap().name, "Researcher");
        assert!(allows(&agent, "web_search"));
        assert!(allows(&agent, "browser__navigate"));
        assert!(!allows(&agent, "bash__run"));
        agent.tools.clear();
        assert!(allows(&agent, "bash__run"));
        assert!(save(dir.path(), Agent { name: " ".into(), ..agent.clone() }).is_err());
        assert!(delete(dir.path(), "a1").unwrap().is_empty());
    }

    #[test]
    fn templates_are_valid() {
        for t in templates() {
            assert!(valid_id(&t.id) && !t.name.is_empty() && !t.instructions.is_empty());
        }
    }
}
