//! Sessions from other coding agents (Vibe Desktop imports projects and sessions from other
//! coding agents, 3.0.0): Claude Code (`~/.claude/projects/<project>/<session>.jsonl`) and Codex
//! (`~/.codex/sessions/**/rollout-*.jsonl`) become Code-mode conversations in their project
//! folder, with what was said kept and each tool step as a short line, so they can be read,
//! searched and continued here.

use crate::support::error::StoreError;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rig_core::completion::Message;
use serde::Serialize;
use serde_json::Value;

use crate::chat::store::{self, Conversation};

/// Characters kept of one message, and messages kept of one session.
const MAX_TEXT: usize = 20_000;
const MAX_MESSAGES: usize = 2_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub path: String,
    /// `claude-code` or `codex`.
    pub source: String,
    pub folder: String,
    pub title: String,
    pub messages: usize,
    pub updated_at: Option<DateTime<Utc>>,
    /// Already imported (the conversation id is derived from the file).
    pub imported: bool,
}

/// One turn read from a session: who spoke, and the text.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub user: bool,
    pub text: String,
    pub at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Session {
    pub folder: String,
    pub turns: Vec<Turn>,
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        return s.to_string();
    }
    let kept: String = crate::support::text::clip(s, MAX_TEXT);
    format!("{kept}\n\n[… cut on import]")
}

/// A tool step as one readable line: `→ Bash: git status`.
fn tool_line(name: &str, input: &Value) -> String {
    let detail = [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
    ]
    .iter()
    .find_map(|k| input.get(*k).and_then(Value::as_str))
    .map(|s| {
        s.lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(160)
            .collect::<String>()
    })
    .unwrap_or_default();
    if detail.is_empty() {
        format!("→ {name}")
    } else {
        format!("→ {name}: {detail}")
    }
}

/// Text of Claude Code message content: strings, text blocks, tool uses as lines; tool results
/// and system reminders are left out.
fn claude_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                Some("tool_use") => Some(tool_line(
                    p.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    p.get("input").unwrap_or(&Value::Null),
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn is_noise(text: &str) -> bool {
    let t = text.trim();
    t.is_empty()
        || t.starts_with("<system-reminder>")
        || t.starts_with("<command-")
        || t.starts_with("<local-command")
        || t.starts_with("Caveat: The messages below")
}

/// A coding agent's session folder: its name, where it is, how deep sessions sit, which files are sessions.
type Source<'a> = (&'a str, PathBuf, usize, &'a dyn Fn(&str) -> bool);

/// Reads a session file of either agent, line by line.
pub(in crate::chat::storage) fn read(path: &Path) -> Result<Session, StoreError> {
    let file = std::fs::File::open(path).map_err(|e| StoreError::read(path, e))?;
    let mut session = Session::default();
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let at = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| t.parse().ok());
        // Claude Code: {"type": "user"|"assistant", "cwd", "message": {"role", "content"}}
        if let (Some(kind @ ("user" | "assistant")), Some(message)) =
            (v.get("type").and_then(Value::as_str), v.get("message"))
        {
            if session.folder.is_empty() {
                session.folder = v
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let text = claude_text(message.get("content").unwrap_or(&Value::Null));
            if !is_noise(&text) {
                push(&mut session.turns, kind == "user", text, at);
            }
            continue;
        }
        // Codex: {"type": "session_meta", "payload": {"cwd"}} and
        // {"type": "response_item", "payload": {"type": "message", "role", "content": [{"text"}]}}
        let payload = v.get("payload").unwrap_or(&Value::Null);
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") if session.folder.is_empty() => {
                session.folder = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            }
            Some("response_item") => match payload.get("type").and_then(Value::as_str) {
                Some("message") => {
                    let role = payload
                        .get("role")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if role != "user" && role != "assistant" {
                        continue;
                    }
                    let text = payload
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    // Codex puts its environment and instructions in user messages too.
                    if !is_noise(&text)
                        && !text.trim_start().starts_with("<environment_context>")
                        && !text.trim_start().starts_with("<user_instructions>")
                    {
                        push(&mut session.turns, role == "user", text, at);
                    }
                }
                Some("function_call") => {
                    let name = payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool");
                    let input = payload
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|a| serde_json::from_str(a).ok())
                        .unwrap_or(Value::Null);
                    let input = match input.get("command") {
                        Some(Value::Array(parts)) => {
                            serde_json::json!({ "command": parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ") })
                        }
                        _ => input,
                    };
                    push(&mut session.turns, false, tool_line(name, &input), at);
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(session)
}

/// Adds a turn, joining it to the last one when the same side speaks again.
fn push(turns: &mut Vec<Turn>, user: bool, text: String, at: Option<DateTime<Utc>>) {
    if let Some(last) = turns.last_mut().filter(|t| t.user == user) {
        last.text.push_str("\n\n");
        last.text.push_str(&text);
        return;
    }
    turns.push(Turn { user, text, at });
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn session_files(root: &Path, depth: usize, wanted: &dyn Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            session_files(&p, depth - 1, wanted, out);
        } else if p.is_file() && wanted(&e.file_name().to_string_lossy()) {
            out.push(p);
        }
    }
}

/// The conversation id an imported file gets: the same file always maps to the same one.
fn id_for(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(path.to_string_lossy().as_bytes());
    let h: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The sessions on this computer, newest first.
pub fn find(config_dir: &Path, home_dir: Option<&Path>) -> Vec<Found> {
    let Some(home) = home_dir.map(Path::to_path_buf).or_else(home) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let sources: [Source; 2] = [
        (
            "claude-code",
            home.join(".claude").join("projects"),
            1,
            &|n: &str| n.ends_with(".jsonl"),
        ),
        (
            "codex",
            home.join(".codex").join("sessions"),
            4,
            &|n: &str| n.starts_with("rollout-") && n.ends_with(".jsonl"),
        ),
    ];
    for (source, root, depth, wanted) in sources {
        let mut files = Vec::new();
        session_files(&root, depth, wanted, &mut files);
        for path in files {
            let Ok(session) = read(&path) else { continue };
            let Some(first) = session.turns.iter().find(|t| t.user) else {
                continue;
            };
            let updated_at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from);
            out.push(Found {
                path: path.to_string_lossy().into_owned(),
                source: source.into(),
                folder: session.folder.clone(),
                title: first
                    .text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect(),
                messages: session.turns.len(),
                updated_at,
                imported: store::load_conversation(config_dir, &id_for(&path)).is_ok(),
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    out
}

/// Imports one session as a Code-mode conversation; again for a file updates the same one.
pub(crate) fn import(config_dir: &Path, path: &Path) -> Result<String, StoreError> {
    let session = read(path)?;
    if !session.turns.iter().any(|t| t.user) {
        return Err(StoreError::Invalid("That session has no messages.".into()));
    }
    let id = id_for(path);
    let mut c = Conversation::new(id.clone(), None, None);
    let source = if path.to_string_lossy().contains(".codex") {
        "Codex"
    } else {
        "Claude Code"
    };
    let first = session
        .turns
        .iter()
        .find(|t| t.user)
        .map(|t| t.text.clone())
        .unwrap_or_default();
    c.title = first
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(60)
        .collect();
    c.tags = vec![source.to_lowercase().replace(' ', "-")];
    c.workdir = Some(session.folder.clone()).filter(|f| !f.is_empty() && Path::new(f).is_dir());
    let turns: Vec<&Turn> = session
        .turns
        .iter()
        .rev()
        .take(MAX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    for t in &turns {
        c.messages.push(if t.user {
            Message::user(clip(&t.text))
        } else {
            Message::assistant(clip(&t.text))
        });
        if let Some(at) = t.at {
            c.times.push((c.messages.len() - 1, at));
        }
    }
    c.notices.push((
        0,
        format!(
            "Imported from {source} ({} messages){}.",
            turns.len(),
            if turns.len() < session.turns.len() {
                ", the latest kept"
            } else {
                ""
            }
        ),
    ));
    if let Some(first_at) = turns.first().and_then(|t| t.at) {
        c.created_at = first_at;
    }
    c.updated_at = turns
        .iter()
        .rev()
        .find_map(|t| t.at)
        .unwrap_or_else(Utc::now);
    store::save_conversation(config_dir, &c)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_sessions_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude/projects/-work-app/s1.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy();
        let lines = [
            r#"{"type":"queue-operation"}"#.to_string(),
            format!(
                r#"{{"type":"user","cwd":"{cwd}","timestamp":"2026-11-01T10:00:00Z","message":{{"role":"user","content":"Fix the failing test"}}}}"#
            ),
            format!(
                r#"{{"type":"user","cwd":"{cwd}","isMeta":true,"message":{{"role":"user","content":"<system-reminder>x</system-reminder>"}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","cwd":"{cwd}","timestamp":"2026-11-01T10:00:05Z","message":{{"role":"assistant","content":[{{"type":"text","text":"Looking."}},{{"type":"tool_use","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}"#
            ),
            format!(
                r#"{{"type":"user","cwd":"{cwd}","message":{{"role":"user","content":[{{"type":"tool_result","content":"ok"}}]}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","cwd":"{cwd}","timestamp":"2026-11-01T10:01:00Z","message":{{"role":"assistant","content":[{{"type":"text","text":"Fixed."}}]}}}}"#
            ),
        ];
        crate::chat::at_rest::write(&file, lines.join("\n")).unwrap();
        let s = read(&file).unwrap();
        assert_eq!(s.folder, cwd);
        assert_eq!(s.turns.len(), 2, "{:?}", s.turns);
        assert!(
            s.turns[1].text.contains("Looking.\n→ Bash: cargo test")
                && s.turns[1].text.ends_with("Fixed.")
        );

        let config = tempfile::tempdir().unwrap();
        let found = find(config.path(), Some(dir.path()));
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                found[0].source.as_str(),
                found[0].title.as_str(),
                found[0].imported
            ),
            ("claude-code", "Fix the failing test", false)
        );
        let id = import(config.path(), &file).unwrap();
        let c = store::load_conversation(config.path(), &id).unwrap();
        assert_eq!(c.workdir.as_deref(), Some(cwd.as_ref()));
        assert_eq!(c.messages.len(), 2);
        assert!(find(config.path(), Some(dir.path()))[0].imported);
        assert_eq!(
            import(config.path(), &file).unwrap(),
            id,
            "importing again updates the same conversation"
        );
    }

    #[test]
    fn codex_sessions_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rollout-2026-11-01.jsonl");
        crate::chat::at_rest::write(&file, [
            r#"{"type":"session_meta","payload":{"cwd":"/repo"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add a README"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"]}"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#,
        ].join("\n")).unwrap();
        let s = read(&file).unwrap();
        assert_eq!(s.folder, "/repo");
        assert_eq!(
            s.turns,
            vec![
                Turn {
                    user: true,
                    text: "Add a README".into(),
                    at: None
                },
                Turn {
                    user: false,
                    text: "→ shell: bash -lc ls\n\nDone.".into(),
                    at: None
                },
            ]
        );
    }
}
