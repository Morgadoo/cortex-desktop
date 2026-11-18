//! Agents that speak the Agent Client Protocol (ACP, from Zed) as models a chat can use, as
//! OpenBot offers: Claude Code (`claude-code-acp`), Codex (`codex-acp`), Gemini CLI
//! (`gemini --experimental-acp`) or any agent the user adds by command. A turn starts the agent,
//! opens a session in the chat's folder and sends the conversation as the prompt over JSON-RPC
//! on stdio; its message and thoughts stream into the chat, its tool calls show as notes, and
//! its permission requests come to the user as approval cards.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::Utc;
use log::{info, warn};
use rig_core::completion::Message;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

use super::engine::{emit, ChatEvent};
use super::providers::ModelView;
use super::store::{self, Conversation};

pub const VENDOR: &str = "acp";
const TIMEOUT_SECS: u64 = 30 * 60;

/// An ACP agent: how to start it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgent {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Agents found on PATH without setup.
const KNOWN: &[(&str, &str, &str, &[&str])] = &[
    ("claude-code", "Claude Code (ACP)", "claude-code-acp", &[]),
    ("codex", "Codex (ACP)", "codex-acp", &[]),
    ("gemini", "Gemini CLI (ACP)", "gemini", &["--experimental-acp"]),
];

fn path(config_dir: &Path) -> PathBuf {
    store::chat_dir(config_dir).join("acp-agents.json")
}

/// The user's own ACP agents.
pub fn load(config_dir: &Path) -> Vec<AcpAgent> {
    crate::chat::at_rest::read_to_string(path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, agents: &[AcpAgent]) -> Result<(), String> {
    for a in agents {
        if !store::valid_id(&a.id) || a.name.trim().is_empty() || a.command.trim().is_empty() {
            return Err("An ACP agent needs a name and a command.".into());
        }
    }
    store::write_json(&path(config_dir), &agents)
}

fn locate(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).flat_map(|d| [d.join(program), d.join(format!("{program}.exe")), d.join(format!("{program}.cmd"))]).find(|p| p.is_file())
}

/// Every ACP agent that can start: the user's, then the known ones installed.
pub fn all(config_dir: &Path) -> Vec<AcpAgent> {
    let mut out = load(config_dir);
    for (id, name, program, args) in KNOWN {
        if !out.iter().any(|a| a.id == *id) && locate(program).is_some() {
            out.push(AcpAgent { id: (*id).into(), name: (*name).into(), command: (*program).into(), args: args.iter().map(|s| s.to_string()).collect() });
        }
    }
    out
}

pub fn models(config_dir: &Path) -> Vec<ModelView> {
    all(config_dir)
        .into_iter()
        .map(|a| ModelView { reference: format!("{VENDOR}/{}", a.id), vendor: VENDOR.into(), provider_label: "ACP agents".into(), id: a.id, name: a.name, reasoning: false, tools: false, context_window: None })
        .collect()
}

pub fn find(config_dir: &Path, reference: &str) -> Option<AcpAgent> {
    let id = reference.strip_prefix(VENDOR)?.strip_prefix('/')?;
    all(config_dir).into_iter().find(|a| a.id == id)
}

/// What a turn hears from the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Text(String),
    Thought(String),
    /// A tool call it started, by title.
    Tool(String),
}

/// The text of an ACP content block.
fn content_text(v: &Value) -> Option<String> {
    (v["type"] == "text").then(|| v["text"].as_str().map(str::to_string)).flatten()
}

/// One `session/update` notification as an event.
pub fn update_event(update: &Value) -> Option<Event> {
    match update["sessionUpdate"].as_str()? {
        "agent_message_chunk" => content_text(&update["content"]).map(Event::Text),
        "agent_thought_chunk" => content_text(&update["content"]).map(Event::Thought),
        "tool_call" => Some(Event::Tool(update["title"].as_str().unwrap_or("A tool").to_string())),
        _ => None,
    }
}

/// The option a permission answer picks: the first `allow_once` (or `allow_always`) when
/// allowed, the first `reject_once` (or `reject_always`) when not.
pub fn pick_option(options: &Value, allow: bool) -> Option<String> {
    let opts = options.as_array()?;
    let kinds: &[&str] = if allow { &["allow_once", "allow_always"] } else { &["reject_once", "reject_always"] };
    kinds.iter().find_map(|k| opts.iter().find(|o| o["kind"] == *k)).and_then(|o| o["optionId"].as_str()).map(str::to_string)
}

/// Runs one prompt with an agent program: initialize, new session, prompt; streams events to
/// `on_event` and asks `permit` about permission requests. Returns the stop reason.
pub async fn drive<F, P, Fut>(program: &Path, args: &[String], cwd: &Path, prompt: &str, mut on_event: F, permit: P, mut cancel: watch::Receiver<bool>) -> Result<String, String>
where
    F: FnMut(Event),
    P: Fn(Value) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("The agent could not start: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no input")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no output")?).lines();
    let stderr = child.stderr.take();
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(mut e) = stderr {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut e, &mut text).await;
        }
        text
    });
    let send = async |msg: Value, stdin: &mut tokio::process::ChildStdin| -> Result<(), String> {
        stdin.write_all(format!("{msg}\n").as_bytes()).await.map_err(|e| format!("The agent stopped listening: {e}"))
    };
    let cwd_text = cwd.to_string_lossy().to_string();
    send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}}}), &mut stdin).await?;
    let mut session: Option<String> = None;
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(TIMEOUT_SECS));
    tokio::pin!(deadline);
    let result = loop {
        let line = tokio::select! {
            l = lines.next_line() => l.map_err(|e| e.to_string())?,
            () = wait_cancel(&mut cancel) => {
                if let Some(s) = &session {
                    let _ = send(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": s}}), &mut stdin).await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                break Ok("cancelled".to_string());
            }
            () = &mut deadline => break Err("The agent ran for 30 minutes and was stopped.".to_string()),
        };
        let Some(line) = line else {
            let detail: String = errors.await.unwrap_or_default().trim().chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(if detail.is_empty() { "The agent closed before answering.".into() } else { format!("The agent closed before answering: {detail}") });
        };
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        // Answers to our requests.
        if msg.get("method").is_none() {
            if let Some(err) = msg.get("error") {
                break Err(format!("The agent refused: {}", err["message"].as_str().unwrap_or("error")));
            }
            match msg["id"].as_u64() {
                Some(1) => send(json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": cwd_text, "mcpServers": []}}), &mut stdin).await?,
                Some(2) => {
                    let s = msg["result"]["sessionId"].as_str().ok_or("The agent opened no session.")?.to_string();
                    send(json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": s, "prompt": [{"type": "text", "text": prompt}]}}), &mut stdin).await?;
                    session = Some(s);
                }
                Some(3) => break Ok(msg["result"]["stopReason"].as_str().unwrap_or("end_turn").to_string()),
                _ => {}
            }
            continue;
        }
        let method = msg["method"].as_str().unwrap_or_default();
        match (method, msg.get("id")) {
            ("session/update", None) => {
                if let Some(e) = update_event(&msg["params"]["update"]) {
                    on_event(e);
                }
            }
            ("session/request_permission", Some(id)) => {
                let allowed = permit(msg["params"].clone()).await;
                let outcome = match pick_option(&msg["params"]["options"], allowed) {
                    Some(option) => json!({"outcome": "selected", "optionId": option}),
                    None => json!({"outcome": "cancelled"}),
                };
                send(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}}), &mut stdin).await?;
            }
            // We offered no file system or terminal; anything else is not supported.
            (_, Some(id)) => send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Not supported by this client"}}), &mut stdin).await?,
            _ => {}
        }
    };
    let _ = child.kill().await;
    result
}

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

/// Runs the turn with an ACP agent; returns whether it was cancelled.
pub async fn run(app: &AppHandle, config_dir: &Path, mut conversation: Conversation, agent: &AcpAgent, cancel: watch::Receiver<bool>) -> Result<bool, String> {
    let id = conversation.id.clone();
    let program = locate(&agent.command).ok_or_else(|| format!("{} is not installed: `{}` was not found.", agent.name, agent.command))?;
    let prompt = super::agent_cli::prompt(&conversation);
    if prompt.trim().is_empty() {
        return Err("There is nothing to send.".into());
    }
    let cwd = match conversation.workdir.as_deref().filter(|d| Path::new(d).is_dir()) {
        Some(d) => PathBuf::from(d),
        None => {
            let d = store::chat_dir(config_dir).join("agent-runs").join(&conversation.id);
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            d
        }
    };
    info!("running the ACP agent {} in {}", agent.name, cwd.display());
    let mut text = String::new();
    let app2 = app.clone();
    let name = agent.name.clone();
    let stop = drive(
        &program,
        &agent.args,
        &cwd,
        &prompt,
        |event| match event {
            Event::Text(t) => {
                text.push_str(&t);
                emit(app, ChatEvent::Text { conversation_id: id.clone(), text: t });
            }
            Event::Thought(t) => emit(app, ChatEvent::Reasoning { conversation_id: id.clone(), text: t }),
            Event::Tool(title) => emit(app, ChatEvent::Notice { conversation_id: id.clone(), text: format!("{name}: {title}") }),
        },
        |params: Value| {
            let app = app2.clone();
            let agent_name = agent.name.clone();
            async move {
                let title = params["toolCall"]["title"].as_str().unwrap_or("an action").to_string();
                let gate = super::tools::ChatTool { name: title.clone(), description: String::new(), parameters: Value::Null, connector_id: Some(agent_name), tool_name: title, read_only: false };
                !matches!(super::tools::request_approval(&app, &gate, &params["toolCall"]).await, crate::tools::registry::ApprovalDecision::Deny)
            }
        },
        cancel,
    )
    .await?;
    let cancelled = stop == "cancelled";
    if !cancelled && text.trim().is_empty() {
        warn!("{} gave no answer (stop: {stop})", agent.name);
        return Err(format!("{} gave no answer ({stop}).", agent.name));
    }
    if !text.trim().is_empty() {
        conversation.messages.push(Message::assistant(text.trim_end().to_string()));
        conversation.times.push((conversation.messages.len() - 1, Utc::now()));
        conversation.updated_at = Utc::now();
        store::save_conversation(config_dir, &conversation)?;
    }
    Ok(cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_and_permissions() {
        assert_eq!(update_event(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hi"}})), Some(Event::Text("Hi".into())));
        assert_eq!(update_event(&json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}})), Some(Event::Thought("hmm".into())));
        assert_eq!(update_event(&json!({"sessionUpdate": "tool_call", "toolCallId": "1", "title": "Read main.rs"})), Some(Event::Tool("Read main.rs".into())));
        assert_eq!(update_event(&json!({"sessionUpdate": "plan", "entries": []})), None);
        let options = json!([{"optionId": "a1", "kind": "allow_always"}, {"optionId": "a0", "kind": "allow_once"}, {"optionId": "r0", "kind": "reject_once"}]);
        assert_eq!(pick_option(&options, true).as_deref(), Some("a0"));
        assert_eq!(pick_option(&options, false).as_deref(), Some("r0"));
        assert_eq!(pick_option(&json!([]), true), None);
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[AcpAgent { id: "mine".into(), name: "Mine".into(), command: "/opt/mine".into(), args: vec![] }]).unwrap();
        assert_eq!(load(dir.path())[0].name, "Mine");
        assert!(save(dir.path(), &[AcpAgent { id: "x".into(), name: " ".into(), command: "c".into(), args: vec![] }]).is_err());
    }

    /// A stand-in agent that speaks the protocol: answers, thinks, asks permission once.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_session_streams_and_asks_permission() {
        let Some(python) = ["python3", "python"].iter().find_map(|p| locate(p)) else { return };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent.py");
        std::fs::write(&script, r#"
import json, sys
def send(m): sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
def upd(s, u): send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": s, "update": u}})
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "initialize": send({"jsonrpc": "2.0", "id": m["id"], "result": {"protocolVersion": 1, "agentCapabilities": {}}})
    elif m.get("method") == "session/new": send({"jsonrpc": "2.0", "id": m["id"], "result": {"sessionId": "s1"}}); cwd = m["params"]["cwd"]
    elif m.get("method") == "session/prompt":
        pid = m["id"]; text = m["params"]["prompt"][0]["text"]
        upd("s1", {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Thinking"}})
        upd("s1", {"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Write notes.md", "kind": "edit"})
        send({"jsonrpc": "2.0", "id": 99, "method": "session/request_permission", "params": {"sessionId": "s1", "toolCall": {"toolCallId": "t1", "title": "Write notes.md"}, "options": [{"optionId": "yes", "name": "Allow", "kind": "allow_once"}, {"optionId": "no", "name": "Reject", "kind": "reject_once"}]}})
    elif m.get("id") == 99:
        choice = m["result"]["outcome"].get("optionId")
        upd("s1", {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "You said: " + text + ". "}})
        upd("s1", {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Permission: " + str(choice) + " in " + cwd}})
        send({"jsonrpc": "2.0", "id": pid, "result": {"stopReason": "end_turn"}})
"#).unwrap();
        let (_tx, rx) = watch::channel(false);
        let mut events = Vec::new();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let asked2 = asked.clone();
        let stop = drive(&python, &[script.to_string_lossy().into_owned()], dir.path(), "hello", |e| events.push(e), move |p: Value| {
            asked2.lock().unwrap().push(p["toolCall"]["title"].as_str().unwrap_or_default().to_string());
            async { true }
        }, rx)
        .await
        .unwrap();
        assert_eq!(stop, "end_turn");
        assert_eq!(asked.lock().unwrap().as_slice(), ["Write notes.md"]);
        assert_eq!(events[0], Event::Thought("Thinking".into()));
        assert_eq!(events[1], Event::Tool("Write notes.md".into()));
        let text: String = events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
        assert!(text.starts_with("You said: hello. Permission: yes in "), "{text}");
    }
}
