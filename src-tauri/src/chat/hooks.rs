//! Hooks: the user's own commands, run at points of a reply as Vibe's and Claude Code's coding
//! agents allow. Kept in `chat/hooks.json`.
//!
//! - `before_tool` runs before a tool call. Exiting with code 2 blocks the call; what the command
//!   printed is the reason the model is told. Any other code lets the call go ahead.
//! - `after_tool` runs after a tool call; what it prints is added to the result the model reads.
//! - `turn_end` runs when a reply finishes (to notify, format code, run tests…).
//!
//! A hook gets the event as JSON on its standard input, and `LUMO_EVENT`, `LUMO_TOOL` and
//! `LUMO_CONVERSATION` in its environment. It runs in the chat's folder when it has one, for at
//! most 60 seconds, with the user's shell.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::store::{chat_dir, write_json};

pub const EVENTS: &[&str] = &["before_tool", "after_tool", "turn_end"];
const TIMEOUT_SECS: u64 = 60;
/// Characters of a hook's output kept.
const MAX_OUTPUT: usize = 4_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hook {
    pub event: String,
    /// Which tools it runs for: empty for all, else names or name prefixes separated by commas
    /// (`run_code, mcp-github`). Not used for `turn_end`.
    #[serde(default)]
    pub matcher: String,
    pub command: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("hooks.json")
}

pub fn load(config_dir: &Path) -> Vec<Hook> {
    std::fs::read_to_string(path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, hooks: &[Hook]) -> Result<(), String> {
    for hook in hooks {
        if !EVENTS.contains(&hook.event.as_str()) {
            return Err(format!("Unknown hook event {}.", hook.event));
        }
        if hook.command.trim().is_empty() {
            return Err("A hook needs a command.".into());
        }
    }
    write_json(&path(config_dir), &hooks)
}

/// Whether a hook's matcher covers a tool.
pub fn matches(matcher: &str, tool: &str) -> bool {
    let parts: Vec<&str> = matcher.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    parts.is_empty() || parts.iter().any(|p| tool == *p || tool.starts_with(p) || (p.ends_with('*') && tool.starts_with(p.trim_end_matches('*'))))
}

/// What a hook did.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub code: Option<i32>,
    pub output: String,
}

/// Runs one hook command with the event on its input.
pub async fn run_command(command: &str, event: &Value, workdir: Option<&Path>) -> Ran {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd.env("LUMO_EVENT", event["event"].as_str().unwrap_or_default());
    cmd.env("LUMO_TOOL", event["tool"].as_str().unwrap_or_default());
    cmd.env("LUMO_CONVERSATION", event["conversationId"].as_str().unwrap_or_default());
    if let Some(dir) = workdir.filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran { code: None, output: format!("The hook could not start: {e}") },
    };
    if let Some(mut stdin) = child.stdin.take() {
        let input = event.to_string();
        let _ = stdin.write_all(input.as_bytes()).await;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(TIMEOUT_SECS), child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !err.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&err);
            }
            Ran { code: out.status.code(), output: text.chars().take(MAX_OUTPUT).collect() }
        }
        Ok(Err(e)) => Ran { code: None, output: e.to_string() },
        Err(_) => Ran { code: None, output: format!("The hook ran past {TIMEOUT_SECS} seconds and was stopped.") },
    }
}

/// Runs the `before_tool` hooks for a call; `Some(reason)` when one of them blocks it.
pub async fn before_tool(config_dir: &Path, conversation_id: &str, tool: &str, arguments: &Value, workdir: Option<&Path>) -> Option<String> {
    let event = json!({ "event": "before_tool", "conversationId": conversation_id, "tool": tool, "arguments": arguments, "workdir": workdir });
    for hook in load(config_dir).iter().filter(|h| h.enabled && h.event == "before_tool" && matches(&h.matcher, tool)) {
        let ran = run_command(&hook.command, &event, workdir).await;
        if ran.code == Some(2) {
            let reason = if ran.output.is_empty() { "A hook blocked this call.".to_string() } else { ran.output };
            log::info!("a before_tool hook blocked {tool}: {reason}");
            return Some(reason);
        }
        if ran.code != Some(0) {
            log::warn!("a before_tool hook for {tool} failed ({:?}): {}", ran.code, ran.output);
        }
    }
    None
}

/// Runs the `after_tool` hooks for a call; what they print, to add to the result.
pub async fn after_tool(config_dir: &Path, conversation_id: &str, tool: &str, arguments: &Value, output: &str, is_error: bool, workdir: Option<&Path>) -> Vec<String> {
    let event = json!({ "event": "after_tool", "conversationId": conversation_id, "tool": tool, "arguments": arguments, "output": output.chars().take(20_000).collect::<String>(), "isError": is_error, "workdir": workdir });
    let mut notes = Vec::new();
    for hook in load(config_dir).iter().filter(|h| h.enabled && h.event == "after_tool" && matches(&h.matcher, tool)) {
        let ran = run_command(&hook.command, &event, workdir).await;
        if !ran.output.is_empty() {
            notes.push(ran.output);
        }
    }
    notes
}

/// Runs the `turn_end` hooks once a reply has finished.
pub async fn turn_end(config_dir: &Path, conversation_id: &str, answer: &str, cancelled: bool, workdir: Option<&Path>) {
    let event = json!({ "event": "turn_end", "conversationId": conversation_id, "answer": answer.chars().take(20_000).collect::<String>(), "cancelled": cancelled, "workdir": workdir });
    for hook in load(config_dir).iter().filter(|h| h.enabled && h.event == "turn_end") {
        let ran = run_command(&hook.command, &event, workdir).await;
        if ran.code != Some(0) {
            log::warn!("a turn_end hook failed ({:?}): {}", ran.code, ran.output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matchers() {
        assert!(matches("", "run_code"));
        assert!(matches("run_code, web_search", "web_search"));
        assert!(matches("mcp-github", "mcp-github__create_issue"));
        assert!(matches("browser*", "browser__navigate"));
        assert!(!matches("run_code", "web_search"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hooks_block_annotate_and_see_the_event() {
        let dir = std::env::temp_dir().join(format!("lumo-hooks-{}", std::process::id()));
        let log = dir.join("events.log");
        std::fs::create_dir_all(&dir).unwrap();
        save(
            &dir,
            &[
                // Blocks deleting commands, says why.
                Hook { event: "before_tool".into(), matcher: "run_code".into(), command: r#"grep -q 'rm -rf' && { echo "No recursive deletes here."; exit 2; } || exit 0"#.into(), enabled: true },
                // A broken hook does not block.
                Hook { event: "before_tool".into(), matcher: String::new(), command: "exit 1".into(), enabled: true },
                Hook { event: "after_tool".into(), matcher: String::new(), command: "echo \"checked $LUMO_TOOL\"".into(), enabled: true },
                Hook { event: "after_tool".into(), matcher: String::new(), command: "echo off".into(), enabled: false },
                Hook { event: "turn_end".into(), matcher: String::new(), command: format!("cat >> '{}'", log.display()), enabled: true },
            ],
        )
        .unwrap();
        let blocked = before_tool(&dir, "c1", "run_code", &json!({"code": "rm -rf /tmp/x"}), None).await;
        assert_eq!(blocked.as_deref(), Some("No recursive deletes here."));
        assert_eq!(before_tool(&dir, "c1", "run_code", &json!({"code": "ls"}), None).await, None);
        assert_eq!(before_tool(&dir, "c1", "web_search", &json!({"query": "rm -rf"}), None).await, None);
        assert_eq!(after_tool(&dir, "c1", "web_search", &json!({}), "ok", false, None).await, ["checked web_search"]);
        turn_end(&dir, "c1", "All done.", false, None).await;
        let logged: Value = serde_json::from_str(std::fs::read_to_string(&log).unwrap().trim()).unwrap();
        assert_eq!(logged["event"], "turn_end");
        assert_eq!(logged["answer"], "All done.");
        assert!(save(&dir, &[Hook { event: "sometime".into(), matcher: String::new(), command: "x".into(), enabled: true }]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
