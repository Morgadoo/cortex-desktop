//! Background commands for coding sessions, as in Vibe Desktop's integrated terminal.
//!
//! A dev server, a watcher or a long test run keeps going while the conversation goes on. Each
//! runs in the session's folder through the shell, its output (stdout and stderr, interleaved)
//! kept in a ring buffer; the user follows it in the Terminal panel and the model reads it with
//! `terminal_read`. Commands stop when asked, and all of them when the app quits.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::tools::ChatTool;

pub const START: &str = "terminal_start";
pub const READ: &str = "terminal_read";
pub const STOP: &str = "terminal_stop";

/// Characters of output kept per command.
const OUTPUT_CHARS: usize = 64_000;
/// Commands running at once, across conversations.
const MAX_RUNNING: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandView {
    pub id: String,
    pub conversation_id: String,
    pub command: String,
    pub started_at: DateTime<Utc>,
    /// `running`, `exited` or `stopped`.
    pub status: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

struct Running {
    view: CommandView,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
    pid: Option<u32>,
}

fn registry() -> &'static Mutex<HashMap<String, Running>> {
    static REG: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Running>> {
    registry().lock().unwrap_or_else(|p| p.into_inner())
}

fn append(id: &str, text: &str) {
    if let Some(r) = lock().get_mut(id) {
        r.view.output.push_str(text);
        let len = r.view.output.chars().count();
        if len > OUTPUT_CHARS {
            r.view.output = r.view.output.chars().skip(len - OUTPUT_CHARS).collect();
        }
    }
}

/// The terminal tools, offered in coding sessions.
pub fn definitions() -> Vec<ChatTool> {
    let tool = |name: &str, description: &str, parameters: serde_json::Value, read_only: bool| ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only,
    };
    vec![
        tool(
            START,
            "Start a long-running command in the background in the session's folder: a dev server, \
             a watcher, a long build or test run. It keeps running while you work; read its output \
             with terminal_read. For short commands use the regular shell tool.",
            json!({ "type": "object", "properties": { "command": { "type": "string" } }, "required": ["command"] }),
            false,
        ),
        tool(
            READ,
            "Read the latest output and status of a background command.",
            json!({ "type": "object", "properties": { "id": { "type": "string" }, "chars": { "type": "integer", "default": 4000 } }, "required": ["id"] }),
            true,
        ),
        tool(
            STOP,
            "Stop a background command.",
            json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
            false,
        ),
    ]
}

/// Starts `command` in `dir` for `conversation_id`; returns its id.
pub fn start(conversation_id: &str, dir: &str, command: &str) -> Result<String, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("Give the command to run.".into());
    }
    if lock().values().filter(|r| r.view.status == "running").count() >= MAX_RUNNING {
        return Err(format!("{MAX_RUNNING} commands are already running; stop one first."));
    }
    #[cfg(windows)]
    let mut child = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    child.args(["/C", command]);
    #[cfg(not(windows))]
    let mut child = tokio::process::Command::new("sh");
    #[cfg(not(windows))]
    child.args(["-c", command]);
    let child = child
        .current_dir(dir)
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Its own process group, so stopping it reaches what the shell started (a server, a watcher).
    #[cfg(unix)]
    let child = child.process_group(0);
    let mut child = child.spawn().map_err(|e| format!("Could not start the command: {e}"))?;
    let pid = child.id();
    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
    lock().insert(
        id.clone(),
        Running {
            view: CommandView {
                id: id.clone(),
                conversation_id: conversation_id.to_string(),
                command: command.to_string(),
                started_at: Utc::now(),
                status: "running".into(),
                exit_code: None,
                output: String::new(),
            },
            kill: Some(kill_tx),
            pid,
        },
    );
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    for stream in [stdout.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), stderr.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)].into_iter().flatten() {
        let id = id.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                append(&id, &format!("{line}\n"));
            }
        });
    }
    let watch_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let (status, code) = tokio::select! {
            result = child.wait() => ("exited", result.ok().and_then(|s| s.code())),
            _ = &mut kill_rx => {
                stop_group(pid).await;
                let _ = child.kill().await;
                ("stopped", None)
            }
        };
        if let Some(r) = lock().get_mut(&watch_id) {
            r.view.status = status.into();
            r.view.exit_code = code;
            r.kill = None;
        }
    });
    Ok(id)
}

/// Ends a command's whole process group: TERM first, then KILL for anything still running.
async fn stop_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        let group = format!("-{pid}");
        let _ = tokio::process::Command::new("kill").args(["-TERM", "--", &group]).status().await;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let _ = tokio::process::Command::new("kill").args(["-KILL", "--", &group]).status().await;
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = tokio::process::Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).status().await;
    }
}

/// Ends every running command and what it started, synchronously: the app is quitting.
pub fn stop_all() {
    let pids: Vec<u32> = lock().values().filter(|r| r.view.status == "running").filter_map(|r| r.pid).collect();
    for pid in pids {
        #[cfg(unix)]
        let _ = std::process::Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
        #[cfg(windows)]
        let _ = std::process::Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).status();
    }
}

/// Stops a running command.
pub fn stop(id: &str) -> Result<(), String> {
    let kill = lock().get_mut(id).and_then(|r| r.kill.take());
    match kill {
        Some(tx) => {
            let _ = tx.send(());
            Ok(())
        }
        None => Err("That command is not running.".into()),
    }
}

/// The commands of a conversation, newest first.
pub fn list(conversation_id: &str) -> Vec<CommandView> {
    let mut out: Vec<CommandView> = lock().values().filter(|r| r.view.conversation_id == conversation_id).map(|r| r.view.clone()).collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.started_at));
    out
}

pub fn get(id: &str) -> Option<CommandView> {
    lock().get(id).map(|r| r.view.clone())
}

/// Forgets finished commands of a conversation.
pub fn clear_finished(conversation_id: &str) {
    lock().retain(|_, r| r.view.conversation_id != conversation_id || r.view.status == "running");
}

/// The tail of a command's output, for the model.
pub fn tail(view: &CommandView, chars: usize) -> String {
    let count = view.output.chars().count();
    if count <= chars {
        view.output.clone()
    } else {
        format!("[… {} earlier characters]\n{}", count - chars, view.output.chars().skip(count - chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_in_the_background_and_stops() {
        if cfg!(windows) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let quick = start("conv-t", dir.path().to_str().unwrap(), "echo hello; echo oops >&2; exit 3").unwrap();
        let slow = start("conv-t", dir.path().to_str().unwrap(), "sleep 98765 & while true; do echo tick; sleep 0.1; done").unwrap();
        for _ in 0..50 {
            if get(&quick).is_some_and(|v| v.status == "exited") && get(&slow).is_some_and(|v| v.output.contains("tick")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let q = get(&quick).unwrap();
        assert_eq!((q.status.as_str(), q.exit_code), ("exited", Some(3)));
        assert!(q.output.contains("hello") && q.output.contains("oops"));
        assert_eq!(get(&slow).unwrap().status, "running");
        stop(&slow).unwrap();
        for _ in 0..50 {
            if get(&slow).is_some_and(|v| v.status == "stopped") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(get(&slow).unwrap().status, "stopped");
        // What the shell started is gone too.
        let left = std::process::Command::new("pgrep").args(["-f", "sleep 98765"]).output().map(|o| o.stdout).unwrap_or_default();
        assert!(left.is_empty(), "the background child survived");
        assert!(stop(&slow).is_err());
        assert_eq!(list("conv-t").len(), 2);
        clear_finished("conv-t");
        assert!(list("conv-t").is_empty());
        assert!(start("conv-t", dir.path().to_str().unwrap(), "  ").is_err());
    }
}
