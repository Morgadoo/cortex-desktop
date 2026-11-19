//! Coding agents that run as command-line programs (Claude Code, Codex, Gemini CLI) as models
//! a chat can use, as LobeHub, Osaurus and OpenBot offer. A turn runs the program once,
//! non-interactively, with the conversation so far as its prompt, in the chat's folder (or a
//! folder of its own), and its output streams into the chat as the answer. The program works
//! with its own tools, login and permission settings; Cortex's tools are not offered to it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::Utc;
use log::{info, warn};
use rig_core::completion::Message;
use tauri::AppHandle;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use super::engine::{emit, ChatEvent};
use super::providers::ModelView;
use super::store::{self, Conversation};

/// The vendor part of these models' references: `agentcli/claude-code`.
pub const VENDOR: &str = "agentcli";

pub struct Cli {
    pub id: &'static str,
    pub name: &'static str,
    pub program: &'static str,
}

pub const CLIS: &[Cli] = &[
    Cli {
        id: "claude-code",
        name: "Claude Code",
        program: "claude",
    },
    Cli {
        id: "codex",
        name: "Codex",
        program: "codex",
    },
    Cli {
        id: "gemini",
        name: "Gemini CLI",
        program: "gemini",
    },
];

/// Characters of earlier conversation passed with each prompt.
const HISTORY_CHARS: usize = 40_000;
/// Longest a run may take.
const TIMEOUT_SECS: u64 = 30 * 60;

pub fn find(reference: &str) -> Option<&'static Cli> {
    let id = reference.strip_prefix(VENDOR)?.strip_prefix('/')?;
    CLIS.iter().find(|c| c.id == id)
}

/// Where a program is on `PATH`, if it is installed.
fn locate(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file() || path.with_extension("exe").is_file() || path.with_extension("cmd").is_file()
}

/// The agent programs installed on this computer, as models.
pub fn installed() -> Vec<ModelView> {
    CLIS.iter()
        .filter(|c| locate(c.program).is_some())
        .map(|c| ModelView {
            reference: format!("{VENDOR}/{}", c.id),
            vendor: VENDOR.into(),
            provider_label: "Agent programs".into(),
            id: c.id.into(),
            name: c.name.into(),
            reasoning: false,
            tools: false,
            context_window: None,
        })
        .collect()
}

/// The command line for one non-interactive run.
pub fn args(cli: &Cli, prompt: &str) -> Vec<String> {
    match cli.id {
        "claude-code" => vec![
            "-p".into(),
            prompt.into(),
            "--output-format".into(),
            "text".into(),
        ],
        "codex" => vec!["exec".into(), "--skip-git-repo-check".into(), prompt.into()],
        _ => vec!["-p".into(), prompt.into()],
    }
}

/// The prompt: the conversation so far, then what the user says now.
pub fn prompt(conversation: &Conversation) -> String {
    let Some(last) = store::last_prompt(conversation) else {
        return String::new();
    };
    let now = match &conversation.messages[last] {
        Message::User { content } => content
            .iter()
            .filter_map(|c| match c {
                rig_core::completion::message::UserContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let mut earlier = conversation.clone();
    earlier.messages.truncate(last);
    let history = super::builtins::chat_transcript(&earlier, HISTORY_CHARS);
    if earlier.messages.is_empty() || history.trim().is_empty() {
        now
    } else {
        format!("The conversation so far, for context:\n\n{history}\n\n---\n\nThe user now says:\n\n{now}")
    }
}

/// The folder a run works in: the chat's own folder, or one kept for this conversation.
fn workdir(config_dir: &Path, conversation: &Conversation) -> Result<PathBuf, String> {
    if let Some(dir) = conversation
        .workdir
        .as_deref()
        .filter(|d| Path::new(d).is_dir())
    {
        return Ok(PathBuf::from(dir));
    }
    let dir = store::chat_dir(config_dir)
        .join("agent-runs")
        .join(&conversation.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Runs the turn with the program; returns whether it was cancelled. The user's message is
/// already in the conversation.
pub async fn run(
    app: &AppHandle,
    config_dir: &Path,
    mut conversation: Conversation,
    cli: &Cli,
    mut cancel: watch::Receiver<bool>,
) -> Result<bool, String> {
    let id = conversation.id.clone();
    let program = locate(cli.program).ok_or_else(|| {
        format!(
            "{} is not installed: `{}` was not found on PATH.",
            cli.name, cli.program
        )
    })?;
    let prompt = prompt(&conversation);
    if prompt.trim().is_empty() {
        return Err("There is nothing to send.".into());
    }
    let dir = workdir(config_dir, &conversation)?;
    info!("running {} in {}", cli.name, dir.display());
    let mut child = tokio::process::Command::new(&program)
        .args(args(cli, &prompt))
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{} could not start: {e}", cli.name))?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let mut stderr = child.stderr.take().ok_or("no output")?;
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let mut text = String::new();
    let mut buf = vec![0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(TIMEOUT_SECS));
    tokio::pin!(deadline);
    let mut cancelled = false;
    loop {
        tokio::select! {
            read = stdout.read(&mut buf) => {
                let n = read.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n]);
                // Only whole UTF-8 characters go out; a split one waits for its other bytes.
                let valid = match std::str::from_utf8(&pending) {
                    Ok(_) => pending.len(),
                    Err(e) => e.valid_up_to(),
                };
                let chunk = String::from_utf8_lossy(&pending[..valid]).into_owned();
                pending.drain(..valid);
                if !chunk.is_empty() {
                    text.push_str(&chunk);
                    emit(app, ChatEvent::Text { conversation_id: id.clone(), text: chunk });
                }
            }
            () = wait_cancel(&mut cancel) => {
                cancelled = true;
                let _ = child.kill().await;
                break;
            }
            () = &mut deadline => {
                let _ = child.kill().await;
                emit(app, ChatEvent::Notice { conversation_id: id.clone(), text: format!("{} ran for 30 minutes and was stopped.", cli.name) });
                break;
            }
        }
    }
    let status = child.wait().await.ok();
    let stderr = errors.await.unwrap_or_default();
    if !cancelled && text.trim().is_empty() {
        let detail: String = stderr
            .trim()
            .chars()
            .rev()
            .take(600)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        warn!("{} gave no answer ({status:?}): {detail}", cli.name);
        return Err(if detail.is_empty() {
            format!("{} gave no answer.", cli.name)
        } else {
            format!("{} gave no answer: {detail}", cli.name)
        });
    }
    if !text.trim().is_empty() {
        conversation
            .messages
            .push(Message::assistant(text.trim_end().to_string()));
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
        conversation.updated_at = Utc::now();
        store::save_conversation(config_dir, &conversation)?;
    }
    Ok(cancelled)
}

/// Waits for the turn to be cancelled; never resolves otherwise.
async fn wait_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_command_lines() {
        assert_eq!(find("agentcli/codex").map(|c| c.name), Some("Codex"));
        assert!(find("agentcli/other").is_none());
        assert!(find("openai/codex").is_none());
        let claude = find("agentcli/claude-code").unwrap();
        assert_eq!(args(claude, "hi"), ["-p", "hi", "--output-format", "text"]);
        assert_eq!(
            args(find("agentcli/codex").unwrap(), "hi"),
            ["exec", "--skip-git-repo-check", "hi"]
        );
        assert_eq!(args(find("agentcli/gemini").unwrap(), "hi"), ["-p", "hi"]);
    }

    #[test]
    fn the_prompt_carries_the_conversation() {
        let mut c = Conversation::new("x".into(), None, None);
        c.messages.push(Message::user("Write a parser"));
        assert_eq!(prompt(&c), "Write a parser");
        c.messages.push(Message::assistant("Done: parser.rs"));
        c.messages.push(Message::user("Now add tests"));
        let p = prompt(&c);
        assert!(p.contains("Write a parser") && p.contains("Done: parser.rs"));
        assert!(p.trim_end().ends_with("Now add tests"));
    }
}
