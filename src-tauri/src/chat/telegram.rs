//! Reach an agent from your phone through Telegram (OpenBot's mobile and chat channels, Osaurus's
//! agent channels): you create a bot with @BotFather, give Lumo its token, and pair your Telegram
//! account with a one-time code. Messages you send the bot become a chat with the chosen agent
//! (or the assistant) in Lumo, and the answer comes back to Telegram.
//!
//! Lumo asks Telegram for new messages (long polling), so nothing has to reach this computer
//! from outside, and it works only while Lumo runs. Only paired accounts are answered; anyone
//! else gets "not paired". Runs are unattended: the assistant reads and searches, but anything
//! that would change something is refused, as in a scheduled task (unless allowed in settings).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

const SECRET: &str = "telegram-bot-token";
/// Telegram's own limit for one message.
const MAX_MESSAGE: usize = 4_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    /// The agent that answers; the general assistant when unset.
    pub agent_id: Option<String>,
    /// Telegram user ids allowed to talk to it.
    pub paired: Vec<i64>,
    /// Their names, for the settings page.
    pub paired_names: Vec<String>,
    /// The code a new account sends to pair, while pairing is open.
    pub pairing_code: Option<String>,
    /// Telegram chat id → Lumo conversation id.
    pub chats: Vec<(i64, String)>,
    /// Let runs make changes (send email, write files) without asking.
    pub allow_changes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    pub has_token: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("telegram.json")
}

pub fn load(config_dir: &Path) -> Settings {
    super::store::read_json(&path(config_dir)).unwrap_or_default()
}

pub fn save(config_dir: &Path, s: &Settings) -> Result<(), String> {
    super::store::write_json(&path(config_dir), s)
}

/// Where Telegram's Bot API is; `LUMO_TELEGRAM_API` points elsewhere for testing.
fn api_base() -> String {
    std::env::var("LUMO_TELEGRAM_API").unwrap_or_else(|_| "https://api.telegram.org".into())
}

pub async fn token() -> Option<String> {
    super::engine::api_key(SECRET).await.ok().flatten().filter(|t| !t.is_empty())
}

pub async fn set_token(token: &str) -> Result<(), String> {
    let account = crate::account::AccountId::parse(super::LOCAL_ACCOUNT)?;
    let secrets = crate::tools::auth::keychain_secrets(account);
    if token.trim().is_empty() {
        secrets.delete(&super::providers::secret_name(SECRET)).await
    } else {
        secrets.store(&super::providers::secret_name(SECRET), token.trim()).await
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap_or_default()
}

async fn call(token: &str, method: &str, body: Value) -> Result<Value, String> {
    let r = client().post(format!("{}/bot{token}/{method}", api_base())).json(&body).send().await.map_err(|e| format!("Telegram did not answer: {e}"))?;
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("Telegram: {}", v.get("description").and_then(Value::as_str).unwrap_or("refused")));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// Checks a token and returns the bot's @name.
pub async fn check(token: &str) -> Result<String, String> {
    let me = call(token, "getMe", json!({})).await?;
    Ok(me.get("username").and_then(Value::as_str).unwrap_or("bot").to_string())
}

/// Splits an answer into Telegram-sized messages, at paragraph or line breaks where it can.
pub fn chunks(text: &str) -> Vec<String> {
    chunks_of(text, MAX_MESSAGE)
}

/// Splits text into pieces of at most `max` characters, at line breaks where it can.
pub fn chunks_of(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if line.chars().count() > max {
            let chars: Vec<char> = line.chars().collect();
            for piece in chars.chunks(max) {
                out.push(piece.iter().collect());
            }
        } else {
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// What to do with one incoming message.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Reply with this and do nothing else.
    Reply(String),
    /// Pair this user (id, name), then reply.
    Pair(i64, String),
    /// Run a turn with this text.
    Ask(String),
    /// Start a fresh conversation for this chat.
    Reset,
    Ignore,
}

/// Decides what an update means, given who is paired and the open pairing code.
pub fn decide(settings: &Settings, user_id: i64, user_name: &str, text: &str) -> Action {
    let text = text.trim();
    if text.is_empty() {
        return Action::Ignore;
    }
    if let Some(code) = text.strip_prefix("/pair").map(str::trim) {
        return match &settings.pairing_code {
            Some(expected) if expected == code => Action::Pair(user_id, user_name.to_string()),
            _ => Action::Reply("That code is not right, or pairing is closed. Open Lumo → Settings → General → Telegram to get a code.".into()),
        };
    }
    if !settings.paired.contains(&user_id) {
        return Action::Reply("This bot answers only its owner. If that is you, send /pair followed by the code shown in Lumo (Settings → General → Telegram).".into());
    }
    match text {
        "/start" | "/help" => Action::Reply("Ask me anything; I answer from Lumo on your computer. /new starts a fresh conversation.".into()),
        "/new" => Action::Reset,
        _ => Action::Ask(text.to_string()),
    }
}

/// Runs one turn for a Telegram chat and returns the answer.
async fn ask(app: &AppHandle, config_dir: &Path, settings: &mut Settings, chat_id: i64, text: &str) -> Result<String, String> {
    let existing = settings.chats.iter().find(|(c, _)| *c == chat_id).map(|(_, id)| id.clone());
    let (id, answer) = run_message(app, config_dir, settings.agent_id.clone(), settings.allow_changes, existing, "Telegram", text).await;
    if let Some(id) = id {
        if !settings.chats.iter().any(|(c, i)| *c == chat_id && *i == id) {
            settings.chats.retain(|(c, _)| *c != chat_id);
            settings.chats.push((chat_id, id));
            save(config_dir, settings)?;
        }
    }
    answer
}

/// Runs a message from a channel (Telegram, Slack) as a turn in a Lumo chat: the chat
/// `existing` when it still exists, else a new one titled `title`. Returns the chat's id and the
/// answer. Unattended unless `allow_changes`: nobody can approve from the channel.
pub async fn run_message(app: &AppHandle, config_dir: &Path, agent_id: Option<String>, allow_changes: bool, existing: Option<String>, title: &str, text: &str) -> (Option<String>, Result<String, String>) {
    use super::engine::{run_turn, TurnRequest};
    let agent = agent_id.as_deref().and_then(|a| super::agents::find(config_dir, a));
    let Some(model) = agent.as_ref().and_then(|a| a.model.clone()).or(super::providers::load(config_dir).default_model) else {
        return (None, Err("Choose a default model in Lumo first.".into()));
    };
    let id = match existing.filter(|id| super::store::load_conversation(config_dir, id).is_ok()) {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let mut c = super::store::Conversation::new(id.clone(), None, Some(model.clone()));
            c.agent_id = agent_id.clone();
            c.title = title.into();
            if let Err(e) = super::store::save_conversation(config_dir, &c) {
                return (None, Err(e));
            }
            id
        }
    };
    let Some(cancel) = app.state::<super::ChatState>().begin(&id) else { return (Some(id), Err("I am still answering your last message.".into())) };
    let request = TurnRequest {
        conversation_id: id.clone(),
        text: text.to_string(),
        model,
        speed: if agent.as_ref().is_some_and(|a| a.think) { super::engine::Speed::Think } else { super::engine::Speed::Fast },
        approval: if allow_changes { super::tools::ApprovalMode::Autonomous } else { super::tools::ApprovalMode::Unattended },
        project_id: None,
        agent_id,
        images: Vec::new(),
        edit_index: None,
        temporary: false,
        resume: false,
        workdir: None,
        connectors: None,
        route_note: Some(format!("Sent from {title}")),
        tools_off: Vec::new(),
    };
    run_turn(app.clone(), request, cancel).await;
    let answer = match super::store::load_conversation(config_dir, &id) {
        Ok(c) => match c.messages.last() {
            Some(m @ rig_core::completion::Message::Assistant(_)) => Ok(super::context::message_text(m)),
            _ => Err("I could not answer that; see the chat in Lumo for what went wrong.".into()),
        },
        Err(e) => Err(e),
    };
    (Some(id), answer)
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Starts listening when Telegram is set up (at start, and after the settings change).
pub fn start(app: &AppHandle) {
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut offset: i64 = 0;
        loop {
            let Ok(config_dir) = crate::settings::get_config_dir(&app) else { break };
            let settings = load(&config_dir);
            let Some(token) = (if settings.enabled { token().await } else { None }) else {
                break;
            };
            let updates = match call(&token, "getUpdates", json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message"] })).await {
                Ok(u) => u,
                Err(e) => {
                    log::warn!("telegram: {e}");
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    continue;
                }
            };
            for update in updates.as_array().cloned().unwrap_or_default() {
                offset = offset.max(update.get("update_id").and_then(Value::as_i64).unwrap_or(0) + 1);
                let Some(message) = update.get("message") else { continue };
                let chat_id = message.pointer("/chat/id").and_then(Value::as_i64).unwrap_or(0);
                let user_id = message.pointer("/from/id").and_then(Value::as_i64).unwrap_or(0);
                let name = message.pointer("/from/first_name").and_then(Value::as_str).unwrap_or("someone").to_string();
                let text = message.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
                let mut settings = load(&config_dir);
                let send = |t: String| {
                    let token = token.clone();
                    async move {
                        for piece in chunks(&t) {
                            if let Err(e) = call(&token, "sendMessage", json!({ "chat_id": chat_id, "text": piece })).await {
                                log::warn!("telegram: could not send: {e}");
                            }
                        }
                    }
                };
                match decide(&settings, user_id, &name, &text) {
                    Action::Ignore => {}
                    Action::Reply(t) => send(t).await,
                    Action::Pair(id, who) => {
                        if !settings.paired.contains(&id) {
                            settings.paired.push(id);
                            settings.paired_names.push(who.clone());
                        }
                        settings.pairing_code = None;
                        let _ = save(&config_dir, &settings);
                        send(format!("Paired, {who}. Ask me anything.")).await;
                    }
                    Action::Reset => {
                        settings.chats.retain(|(c, _)| *c != chat_id);
                        let _ = save(&config_dir, &settings);
                        send("Started a fresh conversation.".into()).await;
                    }
                    Action::Ask(t) => {
                        let _ = call(&token, "sendChatAction", json!({ "chat_id": chat_id, "action": "typing" })).await;
                        let answer = ask(&app, &config_dir, &mut settings, chat_id, &t).await.unwrap_or_else(|e| e);
                        send(answer).await;
                    }
                }
            }
        }
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paired_people_are_answered() {
        let mut s = Settings { pairing_code: Some("482913".into()), ..Default::default() };
        assert!(matches!(decide(&s, 7, "Ana", "hello"), Action::Reply(t) if t.contains("only its owner")));
        assert!(matches!(decide(&s, 7, "Ana", "/pair 111111"), Action::Reply(_)));
        assert_eq!(decide(&s, 7, "Ana", "/pair 482913"), Action::Pair(7, "Ana".into()));
        s.paired.push(7);
        s.pairing_code = None;
        assert_eq!(decide(&s, 7, "Ana", "What's on today?"), Action::Ask("What's on today?".into()));
        assert_eq!(decide(&s, 7, "Ana", "/new"), Action::Reset);
        assert!(matches!(decide(&s, 8, "Eve", "/pair 482913"), Action::Reply(_)), "pairing closed after use");
        assert_eq!(decide(&s, 7, "Ana", "  "), Action::Ignore);
    }

    #[test]
    fn long_answers_are_split() {
        let text = format!("{}\n{}", "a".repeat(3_000), "b".repeat(3_000));
        let parts = chunks(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_MESSAGE));
        assert_eq!(chunks(&"x".repeat(9_000)).len(), 3);
    }
}
