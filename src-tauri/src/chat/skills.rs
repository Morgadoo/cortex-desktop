//! Skills: reusable instructions the assistant loads when a task calls for them.
//!
//! A skill is a folder holding `SKILL.md`, whose YAML front matter names it and says when to use
//! it, followed by the instructions themselves:
//!
//! ```text
//! ---
//! name: challenge-my-thinking
//! description: Use when the user wants their reasoning pressure-tested.
//! ---
//! Steelman the idea first, then…
//! ```
//!
//! The model sees only each enabled skill's name and description, and reads the body with
//! `use_skill` when it decides one applies, so many skills cost little context. The user's skills
//! live in `<config>/skills/<name>/`; a few ship with the app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::write_json;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    /// `user` or `builtin`.
    pub origin: &'static str,
    pub enabled: bool,
}

const BUILTIN: &[(&str, &str)] = &[
    (
        "challenge-my-thinking",
        "---\nname: challenge-my-thinking\ndescription: Use when the user wants to pressure-test an idea, plan or argument (\"challenge this\", \"poke holes\", \"what am I missing\").\n---\n\
1. Restate the idea in its strongest form, in two sentences, so the user knows you understood it.\n\
2. List the 3 to 5 most important weaknesses: hidden assumptions, missing evidence, second-order effects, and how it fails. Rank them by impact.\n\
3. For each, say what evidence would settle it and the cheapest way to get that evidence.\n\
4. Offer the best alternative or a modification that keeps what is good about the idea.\n\
5. End with one question that would change your assessment the most.\n\
Be direct and specific; do not soften every point.",
    ),
    (
        "summarize-document",
        "---\nname: summarize-document\ndescription: Use when the user asks to summarize a document, web page, file or long text.\n---\n\
Read the whole source first (use the file or web tools when it is not in the conversation).\n\
Reply with:\n\
- **TL;DR**: one or two sentences.\n\
- **Key points**: 3 to 7 bullets, each a complete claim, in the source's order.\n\
- **Numbers and dates** that matter, if any.\n\
- **Open questions or caveats** the source leaves.\n\
Quote sparingly and link the source. Match the user's language.",
    ),
    (
        "write-email",
        "---\nname: write-email\ndescription: Use when the user wants an email or message drafted or improved.\n---\n\
Ask for the recipient and the goal only if they cannot be inferred.\n\
Write a subject line, then the email: open with the point, keep paragraphs to 2 or 3 sentences, end with one clear ask and a deadline if relevant.\n\
Match the formality to the recipient. Offer a shorter variant when the draft is over 150 words.",
    ),
    (
        "research-report",
        "---\nname: research-report\ndescription: Use when the user asks for research, a comparison or an overview of a topic that needs current sources.\n---\n\
1. Plan 3 to 5 searches that cover the question from different angles; run them with web_search.\n\
2. Open the most authoritative results (official sites, primary sources) to confirm details.\n\
3. Write: a short answer first, then sections with headings, a comparison table when comparing options, and inline citations as Markdown links.\n\
4. Flag conflicting sources and anything you could not verify.",
    ),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct SkillState {
    /// Skill name → enabled. Missing means enabled.
    enabled: BTreeMap<String, bool>,
}

pub fn skills_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

fn state_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("skills.json")
}

fn load_state(config_dir: &Path) -> SkillState {
    std::fs::read_to_string(state_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Names are folder names and tool arguments: lowercase letters, digits and hyphens.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// `name`, `description` and the body of a `SKILL.md`.
pub fn parse(text: &str) -> Option<(String, String, String)> {
    let rest = text.trim_start_matches('\u{feff}').trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (front, body) = rest.split_at(end);
    let body = body.trim_start_matches("\n---").trim().to_string();
    let mut name = None;
    let mut description = None;
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(['"', '\'']).to_string();
        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    Some((name?, description.unwrap_or_default(), body))
}

pub fn render(name: &str, description: &str, body: &str) -> String {
    let description = description.replace('\n', " ");
    format!("---\nname: {name}\ndescription: {description}\n---\n\n{}\n", body.trim())
}

pub fn list(config_dir: &Path) -> Vec<Skill> {
    let state = load_state(config_dir);
    let enabled = |name: &str| state.enabled.get(name).copied().unwrap_or(true);
    let mut skills: Vec<Skill> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir(config_dir)) {
        for entry in entries.filter_map(Result::ok) {
            let Ok(text) = std::fs::read_to_string(entry.path().join("SKILL.md")) else { continue };
            let Some((name, description, body)) = parse(&text) else { continue };
            if !valid_name(&name) {
                continue;
            }
            skills.push(Skill { enabled: enabled(&name), name, description, body, origin: "user" });
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    for (_, text) in BUILTIN {
        if let Some((name, description, body)) = parse(text) {
            // A user skill of the same name replaces the built-in one.
            if !skills.iter().any(|s| s.name == name) {
                skills.push(Skill { enabled: enabled(&name), name, description, body, origin: "builtin" });
            }
        }
    }
    skills
}

pub fn find(config_dir: &Path, name: &str) -> Option<Skill> {
    list(config_dir).into_iter().find(|s| s.name == name)
}

pub fn save(config_dir: &Path, name: &str, description: &str, body: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err("A skill name uses lowercase letters, digits and hyphens, like `weekly-report`.".into());
    }
    if description.trim().is_empty() {
        return Err("Describe when the assistant should use this skill.".into());
    }
    let dir = skills_dir(config_dir).join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("SKILL.md"), render(name, description, body)).map_err(|e| e.to_string())
}

pub fn delete(config_dir: &Path, name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err("invalid skill name".into());
    }
    let dir = skills_dir(config_dir).join(name);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_enabled(config_dir: &Path, name: &str, enabled: bool) -> Result<(), String> {
    let mut state = load_state(config_dir);
    state.enabled.insert(name.to_string(), enabled);
    write_json(&state_path(config_dir), &state)
}

/// Imports a `SKILL.md`, or a folder holding one, by copying its text.
pub fn import(config_dir: &Path, source: &Path) -> Result<String, String> {
    let file = if source.is_dir() { source.join("SKILL.md") } else { source.to_path_buf() };
    let text = std::fs::read_to_string(&file).map_err(|e| format!("could not read {file:?}: {e}"))?;
    let (name, description, body) =
        parse(&text).ok_or("That file has no `---` front matter with a name and description.")?;
    save(config_dir, &name, &description, &body)?;
    Ok(name)
}

/// The part of the system prompt that lists the enabled skills.
pub fn prompt(skills: &[Skill]) -> Option<String> {
    let enabled: Vec<&Skill> = skills.iter().filter(|s| s.enabled).collect();
    if enabled.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\nSkills: when a request matches one of these, call `use_skill` with its name before \
         answering and follow the instructions it returns.",
    );
    for s in enabled {
        out.push_str(&format!("\n- {}: {}", s.name, s.description));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_files_round_trip() {
        let text = render("weekly-report", "Use for the Friday report.", "Collect wins.\nList risks.");
        let (name, description, body) = parse(&text).unwrap();
        assert_eq!(name, "weekly-report");
        assert_eq!(description, "Use for the Friday report.");
        assert_eq!(body, "Collect wins.\nList risks.");
        assert!(parse("no front matter").is_none());
    }

    #[test]
    fn builtins_parse_and_user_skills_override_them() {
        let dir = tempfile::tempdir().unwrap();
        let skills = list(dir.path());
        assert_eq!(skills.len(), BUILTIN.len());
        assert!(skills.iter().all(|s| s.origin == "builtin" && s.enabled && valid_name(&s.name)));

        save(dir.path(), "write-email", "Mine.", "My way.").unwrap();
        let skills = list(dir.path());
        assert_eq!(skills.len(), BUILTIN.len());
        let mine = skills.iter().find(|s| s.name == "write-email").unwrap();
        assert_eq!((mine.origin, mine.body.as_str()), ("user", "My way."));

        set_enabled(dir.path(), "write-email", false).unwrap();
        assert!(!find(dir.path(), "write-email").unwrap().enabled);
        assert!(!prompt(&list(dir.path())).unwrap().contains("write-email"));

        delete(dir.path(), "write-email").unwrap();
        assert_eq!(find(dir.path(), "write-email").unwrap().origin, "builtin");
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("a-b-1"));
        assert!(!valid_name("A b"));
        assert!(!valid_name("../x"));
        let dir = tempfile::tempdir().unwrap();
        assert!(save(dir.path(), "Bad Name", "d", "b").is_err());
        assert!(save(dir.path(), "ok", " ", "b").is_err());
    }

    #[test]
    fn importing_reads_a_folder_or_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("SKILL.md"), render("imported", "Use it.", "Body")).unwrap();
        assert_eq!(import(dir.path(), source.path()).unwrap(), "imported");
        assert_eq!(find(dir.path(), "imported").unwrap().body, "Body");
    }
}
