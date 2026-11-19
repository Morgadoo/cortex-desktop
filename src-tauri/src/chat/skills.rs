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
//!
//! A skill's folder can also hold supporting files next to `SKILL.md` (templates, reference
//! notes, scripts), as Claude's and Open WebUI's skills do. The model is told their names when it
//! loads the skill, and reads one with `use_skill` and its `file` argument.

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
    /// Supporting files in the skill's folder, as paths relative to it.
    pub files: Vec<String>,
}

const BUILTIN: &[(&str, &str)] = &[
    (
        "data-analysis",
        "---\nname: data-analysis\ndescription: Use when the user wants to inspect, clean, transform, aggregate or chart data, or find anomalies in a spreadsheet, CSV or table.\n---\n1. Load the data with run_code (pandas when available) and report its shape, columns, types and missing values.\n2. Ask what question the data should answer if it is not clear.\n3. Clean only what the question needs, and say what you changed (dropped rows, parsed dates, fixed types).\n4. Compute the aggregates or tests that answer the question; look for outliers and anomalies and explain them.\n5. Make one or two clear charts saved as images, and summarize the findings with the numbers that support them.",
    ),
    (
        "doc-coauthoring",
        "---\nname: doc-coauthoring\ndescription: Use when co-writing a proposal, technical spec, decision document or RFC with the user.\n---\n1. Agree on the document's audience, goal and length before drafting; ask with ask_user if they are unclear.\n2. Propose an outline and wait for the user's go-ahead.\n3. Draft in a canvas, section by section, marking open questions inline as **[Open: …]**.\n4. When the user edits the canvas, keep their changes and build on them; never silently rewrite what they wrote.\n5. End each round with the open questions and the next section to write.",
    ),
    (
        "document-review",
        "---\nname: document-review\ndescription: Use when checking a document for completeness, compliance, consistency or quality.\n---\n1. Ask what the document will be judged against (a template, policy, checklist or audience) unless given.\n2. Read the whole document before commenting.\n3. Report findings in a table: location, issue, severity (blocker, major, minor), suggested fix.\n4. Check consistency of names, numbers, dates and terminology across sections.\n5. Finish with the three changes that would improve it most.",
    ),
    (
        "internal-comms",
        "---\nname: internal-comms\ndescription: Use for status reports, leadership or third-party updates, FAQs, newsletters and incident reports.\n---\n1. Identify the audience and the one thing they must take away.\n2. Use the right template: status (progress, risks, asks), leadership update (headline, impact, decision needed), FAQ (question and answer pairs), incident report (summary, impact, timeline, cause, actions).\n3. Pull facts from the conversation, connected sources and libraries; mark anything unverified.\n4. Write it in a canvas, short and skimmable, with the most important point first.",
    ),
    (
        "meeting-prep",
        "---\nname: meeting-prep\ndescription: Use when preparing for an upcoming meeting or briefing on a calendar item.\n---\n1. Find what the meeting is about: the invite, related mail, documents and earlier notes, using the connectors available.\n2. Summarize the context, the attendees and what each likely wants.\n3. Propose an agenda with the decisions to reach and the questions to ask.\n4. List what the user should read or bring, with links.",
    ),
    (
        "research-synthesis",
        "---\nname: research-synthesis\ndescription: Use when combining several sources, files or notes into one structured brief.\n---\n1. List the sources and what each covers.\n2. Extract the key claims from each, with where they come from.\n3. Group them into themes; note where sources agree, disagree or leave gaps.\n4. Write a brief: summary, themes with citations, contradictions, open questions.",
    ),
    (
        "skill-creator",
        "---\nname: skill-creator\ndescription: Use when the user wants to create, update or delete a skill, or to turn a way of working into a reusable skill.\n---\n1. Ask what the skill is for and when it should be used, unless that is already clear.\n2. Write the description as a trigger that says when to use it (\"Use when …\"), not what it does.\n3. Write focused, numbered instructions; one skill does one job.\n4. Show the draft, take changes, then save it with save_skill. To update, save under the same name.\n5. Tell the user they can call it with /name and manage it under Context, then Skills.",
    ),
    (
        "stakeholder-translator",
        "---\nname: stakeholder-translator\ndescription: Use when reframing content for a different audience, such as executives, engineers, customers or legal.\n---\n1. Ask who the new audience is and what they care about, unless given.\n2. Keep the facts; change the framing, vocabulary, length and level of detail.\n3. For executives lead with impact and decisions; for engineers with specifics and constraints; for customers with benefits and next steps.\n4. Point out anything in the original the new audience would misread.",
    ),
    (
        "structured-extraction",
        "---\nname: structured-extraction\ndescription: Use when pulling tables, fields or JSON out of emails, PDFs or free text.\n---\n1. Agree on the fields to extract and their types; propose a schema if none is given.\n2. Extract every record, keeping values exactly as written and normalizing only formats (dates, numbers) the user asked for.\n3. Mark missing or ambiguous values explicitly instead of guessing.\n4. Return a Markdown table or a JSON block, and save a CSV with run_code when the data is large.",
    ),
    (
        "getting-started",
        "---\nname: getting-started\ndescription: Use when a first-time user asks what this assistant can do, or wants a tour or to set it up.\n---\n1. Explain briefly what it can do here: chat in Work, coding sessions in Code, agents, skills, knowledge, libraries, connectors, scheduled tasks, canvases, mini apps, code runs and images.\n2. Ask with ask_user what they want to use it for first.\n3. Offer to set up what fits: knowledge about them, a project, a library, a connector or a scheduled task.\n4. Walk through one real task end to end, then suggest two next steps.",
    ),
    (
        "set-up-my-knowledge",
        "---\nname: set-up-my-knowledge\ndescription: Use when the user wants to set up their knowledge, or says \"get to know me\" or \"set up my profile\".\n---\n1. Explain in one sentence that knowledge is what you remember about them across chats, and that they can see and edit it under Context, then Knowledge.\n2. Ask, with ask_user, which areas to cover: work and role, current projects, tools they use, writing style, and personal preferences. Let them pick several.\n3. For each area chosen, ask one or two short questions at a time; do not interrogate.\n4. Save each durable fact with remember, as one short sentence in the third person (\"Works as a product designer at a fintech\").\n5. End with a list of what you saved and how to change it.",
    ),
    (
        "discover-knowledge",
        "---\nname: discover-knowledge\ndescription: Use when the user asks what you know or remember about them, or wants to review their knowledge.\n---\n1. List what you remember about the user, grouped by topic (work, projects, preferences, people, other).\n2. Point out anything that looks outdated, duplicated or contradictory.\n3. Suggest two or three things worth adding that would make answers better.\n4. Ask whether to update or remove anything, and use remember and forget for the changes they confirm.",
    ),
    (
        "sanitize-my-knowledge",
        "---\nname: sanitize-my-knowledge\ndescription: Use when the user wants to clean up, tidy or audit their knowledge, or remove sensitive information from it.\n---\n1. Review every remembered fact.\n2. Flag sensitive items (health, finances, credentials, government ids, other people's private details), duplicates, contradictions and stale facts.\n3. Show a table: fact, problem, proposed action (keep, rewrite, merge, forget).\n4. Ask for confirmation with ask_user before changing anything, then apply the approved actions with remember and forget.\n5. Report what changed.",
    ),
    (
        "build-knowledge-from-connectors",
        "---\nname: build-knowledge-from-connectors\ndescription: Use when the user asks to learn about them from their connected apps, files or mail, or to build their knowledge from connectors.\n---\n1. Check which connectors are available in this conversation. If none can read their data, say how to connect one (Context, then Connectors) and stop.\n2. Read a small, recent sample from each: documents in their folders, recent mail subjects, calendar or project names. Read only; change nothing.\n3. Extract durable, useful facts: role, projects, recurring collaborators, tools and preferences. Skip one-off details and anything sensitive.\n4. Show the proposed facts and ask with ask_user which to keep.\n5. Save the approved ones with remember.",
    ),
    (
        "create-skill-from-knowledge",
        "---\nname: create-skill-from-knowledge\ndescription: Use when the user wants to turn what you know about them, or a way of working they repeat, into a reusable skill.\n---\n1. Ask what the skill should do and when it should be used, unless that is already clear.\n2. Draft it from their knowledge and this conversation: a lowercase hyphenated name, a description that says when to use it, and numbered instructions.\n3. Show the draft and ask for changes.\n4. Save it with save_skill once they approve, and tell them they can call it with /name or find it under Context, then Skills.",
    ),
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
        "deep-research",
        "---\nname: deep-research\ndescription: Use when the user asks for research, a comparison, a market or state-of-the-art overview, or any question that needs many current sources.\n---\n\
1. Say in one or two sentences how you will approach it, then call update_plan with 4 to 6 research areas.\n\
2. Hand the areas to delegate, one task each, so sub-agents research them in parallel. Write every task so it stands alone: what to find, which kinds of sources to prefer, and what to report.\n\
3. While they work, or after, run a few web_search calls yourself for anything cross-cutting, and open primary sources to confirm key figures.\n\
4. Mark the plan's steps done as results come in.\n\
5. Write the report: a short answer first, then one section per area with headings, a table when comparing options, inline citations as Markdown links, and a closing section on open questions and what you could not verify.",
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
    crate::chat::at_rest::read_to_string(state_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Names are folder names and tool arguments: lowercase letters, digits and hyphens.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// `name`, `description` and the body of a `SKILL.md`.
pub fn parse(text: &str) -> Option<(String, String, String)> {
    let rest = text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (front, body) = rest.split_at(end);
    let body = body.trim_start_matches("\n---").trim().to_string();
    let mut name = None;
    let mut description = None;
    let lines: Vec<&str> = front.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        // Top-level keys only; indented lines belong to the value before them.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let raw = value.trim();
        // YAML block scalars (`>` folds lines, `|` keeps them): the indented lines that follow.
        let value = if raw.starts_with('>') || raw.starts_with('|') {
            let mut block = Vec::new();
            while i < lines.len()
                && (lines[i].starts_with([' ', '\t']) || lines[i].trim().is_empty())
            {
                block.push(lines[i].trim());
                i += 1;
            }
            if raw.starts_with('>') {
                block
                    .iter()
                    .filter(|l| !l.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                block.join("\n").trim().to_string()
            }
        } else {
            raw.trim_matches(['"', '\'']).to_string()
        };
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
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
        body.trim()
    )
}

pub fn list(config_dir: &Path) -> Vec<Skill> {
    let state = load_state(config_dir);
    let enabled = |name: &str| state.enabled.get(name).copied().unwrap_or(true);
    let mut skills: Vec<Skill> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir(config_dir)) {
        for entry in entries.filter_map(Result::ok) {
            let Ok(text) = crate::chat::at_rest::read_to_string(entry.path().join("SKILL.md"))
            else {
                continue;
            };
            let Some((name, description, body)) = parse(&text) else {
                continue;
            };
            if !valid_name(&name) {
                continue;
            }
            let files = supporting_files(&entry.path());
            skills.push(Skill {
                enabled: enabled(&name),
                name,
                description,
                body,
                origin: "user",
                files,
            });
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    for (_, text) in BUILTIN {
        if let Some((name, description, body)) = parse(text) {
            // A user skill of the same name replaces the built-in one.
            if !skills.iter().any(|s| s.name == name) {
                skills.push(Skill {
                    enabled: enabled(&name),
                    name,
                    description,
                    body,
                    origin: "builtin",
                    files: Vec::new(),
                });
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
        return Err(
            "A skill name uses lowercase letters, digits and hyphens, like `weekly-report`.".into(),
        );
    }
    if description.trim().is_empty() {
        return Err("Describe when the assistant should use this skill.".into());
    }
    let dir = skills_dir(config_dir).join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let next = render(name, description, body);
    // The version being replaced is kept, so any earlier one can come back.
    if let Ok(previous) = crate::chat::at_rest::read_to_string(dir.join("SKILL.md")) {
        if previous != next {
            let history = dir.join("history");
            std::fs::create_dir_all(&history).map_err(|e| e.to_string())?;
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
            // Two saves in the same millisecond still keep both.
            let mut file = history.join(format!("{stamp}.md"));
            let mut n = 1;
            while file.exists() {
                file = history.join(format!("{stamp}-{n}.md"));
                n += 1;
            }
            crate::chat::at_rest::write(file, previous).map_err(|e| e.to_string())?;
            prune_history(&history);
        }
    }
    crate::chat::at_rest::write(dir.join("SKILL.md"), next).map_err(|e| e.to_string())
}

/// Versions kept per skill.
const KEEP_VERSIONS: usize = 30;

fn prune_history(history: &Path) {
    let mut files: Vec<_> = std::fs::read_dir(history)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    let extra = files.len().saturating_sub(KEEP_VERSIONS);
    for old in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

/// An earlier version of a skill.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// When it was replaced (its file name, sortable).
    pub id: String,
    pub description: String,
    pub body: String,
}

/// A skill's earlier versions, newest first.
pub fn history(config_dir: &Path, name: &str) -> Result<Vec<Version>, String> {
    if !valid_name(name) {
        return Err("invalid skill name".into());
    }
    let dir = skills_dir(config_dir).join(name).join("history");
    let mut out: Vec<Version> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.path().file_stem()?.to_string_lossy().into_owned();
            let (_, description, body) =
                parse(&crate::chat::at_rest::read_to_string(e.path()).ok()?)?;
            Some(Version {
                id,
                description,
                body,
            })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// Brings an earlier version back (the current one joins the history).
pub fn restore(config_dir: &Path, name: &str, version: &str) -> Result<(), String> {
    let found = history(config_dir, name)?
        .into_iter()
        .find(|v| v.id == version)
        .ok_or("That version is gone.")?;
    save(config_dir, name, &found.description, &found.body)
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

/// Imports a `SKILL.md`, or a folder holding one with its supporting files.
pub fn import(config_dir: &Path, source: &Path) -> Result<String, String> {
    let file = if source.is_dir() {
        source.join("SKILL.md")
    } else {
        source.to_path_buf()
    };
    let text = crate::chat::at_rest::read_to_string(&file)
        .map_err(|e| format!("could not read {file:?}: {e}"))?;
    let (name, description, body) =
        parse(&text).ok_or("That file has no `---` front matter with a name and description.")?;
    save(config_dir, &name, &description, &body)?;
    if source.is_dir() {
        for rel in supporting_files(source) {
            if let Ok(bytes) = std::fs::read(source.join(&rel)) {
                add_file(config_dir, &name, &rel, &bytes)?;
            }
        }
    }
    Ok(name)
}

/// Supporting files per skill, and the size of each.
const MAX_FILES: usize = 200;
const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
/// Characters of a text file `use_skill` returns.
const MAX_FILE_CHARS: usize = 100_000;

/// A path inside a skill's folder: relative, no `..`, and neither `SKILL.md` nor its history.
fn file_path_ok(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split(['/', '\\']).collect();
    !rel.is_empty()
        && rel.len() <= 200
        && !rel.starts_with(['/', '\\'])
        && !rel.contains(':')
        && parts
            .iter()
            .all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.starts_with('.'))
        && rel != "SKILL.md"
        && parts[0] != "history"
}

/// The supporting files in a skill's folder, sorted, as `/`-separated relative paths.
pub fn supporting_files(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !file_path_ok(&rel) {
                continue;
            }
            if path.is_dir() {
                if depth < 5 {
                    walk(base, &path, depth + 1, out);
                }
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, 0, &mut out);
    out.sort();
    out
}

fn user_skill_dir(config_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err("invalid skill name".into());
    }
    let dir = skills_dir(config_dir).join(name);
    if !dir.join("SKILL.md").is_file() {
        return Err("Save the skill before adding files to it (a built-in skill needs your own copy first).".into());
    }
    Ok(dir)
}

/// Adds or replaces a supporting file.
pub fn add_file(config_dir: &Path, name: &str, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = user_skill_dir(config_dir, name)?;
    let rel = rel.replace('\\', "/");
    if !file_path_ok(&rel) {
        return Err(format!("{rel} cannot be a skill file's name."));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{rel} is over {} MB.",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }
    let files = supporting_files(&dir);
    if files.len() >= MAX_FILES && !files.contains(&rel) {
        return Err(format!("A skill holds at most {MAX_FILES} files."));
    }
    let target = dir.join(&rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::chat::at_rest::write(&target, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if rel.ends_with(".sh") || rel.starts_with("scripts/") || rel.starts_with("bin/") {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

pub fn remove_file(config_dir: &Path, name: &str, rel: &str) -> Result<(), String> {
    let dir = user_skill_dir(config_dir, name)?;
    if !file_path_ok(rel) {
        return Err("No such file.".into());
    }
    std::fs::remove_file(dir.join(rel)).map_err(|e| e.to_string())?;
    // Folders left empty go too.
    let mut parent = dir.join(rel).parent().map(Path::to_path_buf);
    while let Some(p) = parent.filter(|p| p != &dir) {
        if std::fs::remove_dir(&p).is_err() {
            break;
        }
        parent = p.parent().map(Path::to_path_buf);
    }
    Ok(())
}

/// A supporting file's text; a binary file is described with its full path, for tools that run it.
pub fn read_file(config_dir: &Path, name: &str, rel: &str) -> Result<String, String> {
    if !valid_name(name) || !file_path_ok(rel) {
        return Err("No such file in that skill.".into());
    }
    let path = skills_dir(config_dir).join(name).join(rel);
    let bytes =
        crate::chat::at_rest::read(&path).map_err(|_| "No such file in that skill.".to_string())?;
    match String::from_utf8(bytes) {
        Ok(text) if text.chars().count() > MAX_FILE_CHARS => Ok(format!(
            "{}\n\n[cut at {MAX_FILE_CHARS} characters]",
            text.chars().take(MAX_FILE_CHARS).collect::<String>()
        )),
        Ok(text) => Ok(text),
        Err(e) => Ok(format!(
            "{rel} is a binary file ({} bytes) at {}.",
            e.as_bytes().len(),
            path.display()
        )),
    }
}

/// What `use_skill` returns: the instructions, and the files that come with them.
pub fn loaded(skill: &Skill) -> String {
    let mut out = format!("# Skill: {}\n\n{}", skill.name, skill.body);
    if !skill.files.is_empty() {
        out.push_str("\n\n## Supporting files\nRead one with `use_skill` and its `file` argument when the instructions call for it:");
        for f in &skill.files {
            out.push_str(&format!("\n- {f}"));
        }
    }
    out
}

/// Where to fetch a skill's `SKILL.md` from a link to it: a GitHub folder or file (tree, blob or
/// raw), a repository with the skill at its root, or any `https` link to a Markdown file.
pub fn skill_sources(link: &str) -> Result<Vec<String>, String> {
    let link = link.trim().trim_end_matches('/');
    let url = url::Url::parse(link).map_err(|_| "That is not a link.".to_string())?;
    if url.scheme() != "https" {
        return Err("Use an https link.".into());
    }
    let parts: Vec<&str> = url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    let skill_md = |base: String| {
        if base.to_lowercase().ends_with(".md") {
            base
        } else {
            format!("{base}/SKILL.md")
        }
    };
    match url.host_str() {
        Some("github.com") => match parts.as_slice() {
            [owner, repo] => Ok(["HEAD", "main", "master"]
                .iter()
                .map(|r| format!("https://raw.githubusercontent.com/{owner}/{repo}/{r}/SKILL.md"))
                .collect()),
            [owner, repo, "tree" | "blob", reference, rest @ ..] => Ok(vec![skill_md(format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{}",
                rest.join("/")
            ))]),
            _ => Err("Link to the skill's folder or its SKILL.md on GitHub.".into()),
        },
        Some("raw.githubusercontent.com") => Ok(vec![skill_md(link.to_string())]),
        Some(_) if link.to_lowercase().ends_with(".md") => Ok(vec![link.to_string()]),
        _ => Err("Link to a SKILL.md file, or to a skill's folder on GitHub.".into()),
    }
}

/// Downloads a skill from a link and saves it; returns its name. Only the instructions come
/// along: scripts and other files in the skill's folder are not downloaded.
pub async fn import_from_link(config_dir: &Path, link: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut last = String::from("No SKILL.md was found there.");
    for source in skill_sources(link)? {
        let response = match client.get(&source).send().await {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        if !response.status().is_success() {
            last = format!("{source} answered {}.", response.status());
            continue;
        }
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        if bytes.len() > 512 * 1024 {
            return Err("That file is too large for a skill.".into());
        }
        let text = String::from_utf8_lossy(&bytes);
        let (name, description, body) = parse(&text)
            .ok_or("That file has no `---` front matter with a name and description.")?;
        save(config_dir, &name, &description, &body)?;
        return Ok(name);
    }
    Err(last)
}

/// The part of the system prompt that lists the enabled skills.
/// Past this many skills, only the ones that fit the message are listed.
pub const LIST_ALL: usize = 30;
const LIST_RELEVANT: usize = 15;

/// The skills section of the system prompt. With many skills, it names those that fit `query`
/// (the user's message) and says how to find the rest, so the prompt stays the same size however
/// many skills there are.
pub fn prompt_for(skills: &[Skill], query: &str) -> Option<String> {
    let enabled: Vec<&Skill> = skills.iter().filter(|s| s.enabled).collect();
    if enabled.is_empty() {
        return None;
    }
    let mut out = String::from(
        "\n\nSkills: when a request matches one of these, call `use_skill` with its name before \
         answering and follow the instructions it returns.",
    );
    if enabled.len() <= LIST_ALL {
        for s in enabled {
            out.push_str(&format!("\n- {}: {}", s.name, s.description));
        }
        return Some(out);
    }
    let shown: Vec<&Skill> = relevant(&enabled, query)
        .into_iter()
        .take(LIST_RELEVANT)
        .collect();
    for s in &shown {
        out.push_str(&format!("\n- {}: {}", s.name, s.description));
    }
    out.push_str(&format!(
        "\n({} more skills are not listed. Call `use_skill` with a few words instead of a name to \
         find one.)",
        enabled.len() - shown.len()
    ));
    Some(out)
}

/// Skills ranked by how well their name and description fit `query`, best first.
pub fn relevant<'a>(skills: &[&'a Skill], query: &str) -> Vec<&'a Skill> {
    use super::retrieval::{tokens, Corpus};
    let corpus = Corpus::new(skills.iter().map(|s| {
        let mut words = tokens(&s.name.replace('-', " "));
        words.extend(tokens(&s.name.replace('-', " ")));
        words.extend(tokens(&s.description));
        words
    }))
    .with_prefixes();
    corpus
        .search(query)
        .into_iter()
        .map(|(i, _)| skills[i])
        .collect()
}

#[cfg(test)]
mod file_tests {
    use super::*;

    /// With many skills, the prompt names the ones that fit the message and stays short.
    #[test]
    fn many_skills_are_narrowed_to_the_ones_that_fit() {
        let skill = |name: &str, description: &str| Skill {
            name: name.into(),
            description: description.into(),
            body: String::new(),
            origin: "user",
            enabled: true,
            files: Vec::new(),
        };
        let mut skills: Vec<Skill> = (0..40)
            .map(|i| skill(&format!("chore-{i}"), "Tidy something unrelated."))
            .collect();
        skills.push(skill(
            "quarterly-invoice",
            "Write the quarterly invoices for clients.",
        ));
        let few = prompt_for(&skills[..10], "anything").unwrap();
        assert_eq!(few.matches("\n- ").count(), 10);
        let many = prompt_for(&skills, "send the invoices to clients").unwrap();
        assert!(many.contains("quarterly-invoice"), "{many}");
        assert!(many.contains("more skills are not listed"));
        assert!(many.matches("\n- ").count() <= LIST_RELEVANT);
    }

    #[test]
    fn skills_carry_supporting_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            add_file(dir.path(), "report", "template.md", b"x").is_err(),
            "the skill must exist first"
        );
        save(
            dir.path(),
            "report",
            "Use for the weekly report.",
            "Fill templates/week.md.",
        )
        .unwrap();
        add_file(dir.path(), "report", "templates/week.md", b"# Week {{n}}").unwrap();
        add_file(dir.path(), "report", "scripts/count.py", b"print(1)").unwrap();
        add_file(
            dir.path(),
            "report",
            "logo.png",
            &[0x89, b'P', b'N', b'G', 0xff, 0xfe],
        )
        .unwrap();
        for bad in [
            "../escape.md",
            "/etc/passwd",
            "SKILL.md",
            "history/x.md",
            ".hidden",
            "a//b",
            "C:/x",
        ] {
            assert!(add_file(dir.path(), "report", bad, b"x").is_err(), "{bad}");
        }
        let skill = find(dir.path(), "report").unwrap();
        assert_eq!(
            skill.files,
            ["logo.png", "scripts/count.py", "templates/week.md"]
        );
        let text = loaded(&skill);
        assert!(
            text.contains("## Supporting files") && text.contains("- templates/week.md"),
            "{text}"
        );
        assert_eq!(
            read_file(dir.path(), "report", "templates/week.md").unwrap(),
            "# Week {{n}}"
        );
        assert!(read_file(dir.path(), "report", "logo.png")
            .unwrap()
            .contains("binary file (6 bytes)"));
        assert!(read_file(dir.path(), "report", "../report/SKILL.md").is_err());
        // Saving the instructions again keeps the files, and the history stays out of the list.
        save(
            dir.path(),
            "report",
            "Use for the weekly report.",
            "Fill it in.",
        )
        .unwrap();
        assert_eq!(find(dir.path(), "report").unwrap().files.len(), 3);
        remove_file(dir.path(), "report", "templates/week.md").unwrap();
        assert!(
            !dir.path().join("skills/report/templates").exists(),
            "an emptied folder goes"
        );
        assert_eq!(
            find(dir.path(), "report").unwrap().files,
            ["logo.png", "scripts/count.py"]
        );
        // A folder imports with its files.
        let src = dir.path().join("src-skill");
        std::fs::create_dir_all(src.join("refs")).unwrap();
        std::fs::write(
            src.join("SKILL.md"),
            "---\nname: brand-voice\ndescription: Use for copy.\n---\nSee refs/tone.md.",
        )
        .unwrap();
        std::fs::write(src.join("refs/tone.md"), "Warm, short.").unwrap();
        assert_eq!(import(dir.path(), &src).unwrap(), "brand-voice");
        assert_eq!(
            read_file(dir.path(), "brand-voice", "refs/tone.md").unwrap(),
            "Warm, short."
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn links_point_at_the_skill_file() {
        assert_eq!(
            skill_sources("https://github.com/anthropics/skills/tree/main/document-skills/pdf").unwrap(),
            ["https://raw.githubusercontent.com/anthropics/skills/main/document-skills/pdf/SKILL.md"]
        );
        assert_eq!(
            skill_sources("https://github.com/acme/tools/blob/v2/skills/report/SKILL.md").unwrap(),
            ["https://raw.githubusercontent.com/acme/tools/v2/skills/report/SKILL.md"]
        );
        assert_eq!(
            skill_sources("https://github.com/acme/my-skill/")
                .unwrap()
                .len(),
            3,
            "a repository is tried at HEAD, main and master"
        );
        assert_eq!(
            skill_sources("https://example.com/skills/x.md").unwrap(),
            ["https://example.com/skills/x.md"]
        );
        assert!(skill_sources("http://github.com/a/b").is_err());
        assert!(skill_sources("https://example.com/page").is_err());
        assert!(skill_sources("not a link").is_err());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn block_scalar_descriptions_are_read() {
        let text = "---\nname: guide\ndescription: >\n  Use this when\n  asked about courses.\nlicense: MIT\n---\nBody";
        let (name, description, body) = super::parse(text).unwrap();
        assert_eq!(
            (name.as_str(), description.as_str(), body.as_str()),
            ("guide", "Use this when asked about courses.", "Body")
        );
        let kept =
            super::parse("---\nname: k\ndescription: |\n  line one\n  line two\n---\nB").unwrap();
        assert_eq!(kept.1, "line one\nline two");
        assert_eq!(
            super::parse("---\nname: plain\ndescription: \"quoted: yes\"\n---\nx")
                .unwrap()
                .1,
            "quoted: yes"
        );
    }

    #[test]
    fn skills_keep_their_versions() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), "report", "Use when reporting.", "1. Draft.").unwrap();
        save(dir.path(), "report", "Use when reporting.", "1. Draft.").unwrap();
        assert!(
            history(dir.path(), "report").unwrap().is_empty(),
            "saving the same text keeps no copy"
        );
        save(
            dir.path(),
            "report",
            "Use when reporting weekly.",
            "1. Draft.\n2. Send.",
        )
        .unwrap();
        let versions = history(dir.path(), "report").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].body.trim(), "1. Draft.");
        restore(dir.path(), "report", &versions[0].id).unwrap();
        assert_eq!(find(dir.path(), "report").unwrap().body.trim(), "1. Draft.");
        assert_eq!(
            history(dir.path(), "report").unwrap().len(),
            2,
            "the replaced version is kept too"
        );
    }

    use super::*;

    #[test]
    fn skill_files_round_trip() {
        let text = render(
            "weekly-report",
            "Use for the Friday report.",
            "Collect wins.\nList risks.",
        );
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
        assert!(skills
            .iter()
            .all(|s| s.origin == "builtin" && s.enabled && valid_name(&s.name)));

        save(dir.path(), "write-email", "Mine.", "My way.").unwrap();
        let skills = list(dir.path());
        assert_eq!(skills.len(), BUILTIN.len());
        let mine = skills.iter().find(|s| s.name == "write-email").unwrap();
        assert_eq!((mine.origin, mine.body.as_str()), ("user", "My way."));

        set_enabled(dir.path(), "write-email", false).unwrap();
        assert!(!find(dir.path(), "write-email").unwrap().enabled);
        assert!(!prompt_for(&list(dir.path()), "")
            .unwrap()
            .contains("write-email"));

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
        crate::chat::at_rest::write(
            source.path().join("SKILL.md"),
            render("imported", "Use it.", "Body"),
        )
        .unwrap();
        assert_eq!(import(dir.path(), source.path()).unwrap(), "imported");
        assert_eq!(find(dir.path(), "imported").unwrap().body, "Body");
    }
}
