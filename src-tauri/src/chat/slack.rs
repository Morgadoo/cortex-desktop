//! Reach an agent from Slack (OpenBot's Slack channel): a Slack app of your own, connected with
//! Socket Mode, so Slack's events come over a connection Cortex opens. Nothing has to reach this
//! computer from outside, and no public address is needed.
//!
//! Setup: create an app at api.slack.com/apps, turn on Socket Mode (an app-level token, `xapp-…`,
//! with `connections:write`), add the bot scopes `chat:write`, `im:history` and
//! `app_mentions:read`, subscribe to the bot events `message.im` and `app_mention`, install it,
//! and give Cortex both tokens. Direct messages to the app, and mentions of it in a channel, are
//! answered in a thread. Only Slack members paired with a one-time code are answered.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

const APP_TOKEN: &str = "slack-app-token";
const BOT_TOKEN: &str = "slack-bot-token";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub agent_id: Option<String>,
    /// Slack member ids allowed to talk to it.
    pub paired: Vec<String>,
    pub pairing_code: Option<String>,
    /// Slack channel (or DM) id → Cortex conversation id.
    pub chats: Vec<(String, String)>,
    pub allow_changes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub has_tokens: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("slack.json")
}

pub fn load(config_dir: &Path) -> Settings {
    super::store::read_json(&path(config_dir)).unwrap_or_default()
}

pub fn save(config_dir: &Path, s: &Settings) -> Result<(), String> {
    super::store::write_json(&path(config_dir), s)
}

/// Slack's Web API; `CORTEX_SLACK_API` points elsewhere for testing.
fn api_base() -> String {
    std::env::var("CORTEX_SLACK_API").unwrap_or_else(|_| "https://slack.com/api".into())
}

pub async fn tokens() -> Option<(String, String)> {
    let app = super::secrets::api_key(APP_TOKEN)
        .await
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())?;
    let bot = super::secrets::api_key(BOT_TOKEN)
        .await
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())?;
    Some((app, bot))
}

pub async fn set_tokens(app_token: &str, bot_token: &str) -> Result<(), String> {
    let secrets = crate::chat::secrets::local()?;
    for (name, value) in [(APP_TOKEN, app_token), (BOT_TOKEN, bot_token)] {
        if value.trim().is_empty() {
            secrets.delete(&super::providers::secret_name(name)).await?;
        } else {
            secrets
                .store(&super::providers::secret_name(name), value.trim())
                .await?;
        }
    }
    Ok(())
}

async fn call(token: &str, method: &str, body: Value) -> Result<Value, String> {
    let r = crate::support::http::client(Duration::from_secs(30))?
        .post(format!("{}/{method}", api_base()))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Slack did not answer: {e}"))?;
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "Slack: {}",
            v.get("error").and_then(Value::as_str).unwrap_or("refused")
        ));
    }
    Ok(v)
}

/// Checks both tokens; returns the workspace's name.
pub async fn check(app_token: &str, bot_token: &str) -> Result<String, String> {
    if !app_token.starts_with("xapp-") {
        return Err(
            "The app-level token starts with xapp- (Basic Information → App-Level Tokens).".into(),
        );
    }
    if !bot_token.starts_with("xoxb-") {
        return Err("The bot token starts with xoxb- (OAuth & Permissions).".into());
    }
    let who = call(bot_token, "auth.test", json!({})).await?;
    call(app_token, "apps.connections.open", json!({})).await?;
    Ok(who
        .get("team")
        .and_then(Value::as_str)
        .unwrap_or("Slack")
        .to_string())
}

/// A message's text without the mention of the app at its start.
pub fn clean(text: &str) -> String {
    let t = text.trim();
    let t = if t.starts_with("<@") {
        t.split_once('>').map_or(t, |(_, rest)| rest)
    } else {
        t
    };
    t.trim().to_string()
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Reply(String),
    Pair(String),
    Ask(String),
    Reset,
    Ignore,
}

/// What an event means. Messages from bots (including this one) are ignored.
pub fn decide(settings: &Settings, event: &Value) -> Action {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if event.get("bot_id").is_some()
        || event.get("subtype").is_some()
        || !matches!(kind, "message" | "app_mention")
    {
        return Action::Ignore;
    }
    // Direct messages arrive as `message` (channel_type im); in channels, only mentions count.
    if kind == "message" && event.get("channel_type").and_then(Value::as_str) != Some("im") {
        return Action::Ignore;
    }
    let user = event
        .get("user")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let text = clean(
        event
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if text.is_empty() || user.is_empty() {
        return Action::Ignore;
    }
    if let Some(code) = text.strip_prefix("pair").map(str::trim) {
        return match &settings.pairing_code {
            Some(expected) if expected == code => Action::Pair(user),
            _ => Action::Reply("That code is not right, or pairing is closed. Open Cortex → Settings → General → Slack to get a code.".into()),
        };
    }
    if !settings.paired.contains(&user) {
        return Action::Reply("I answer only my owner. If that is you, send `pair` followed by the code shown in Cortex (Settings → General → Slack).".into());
    }
    match text.as_str() {
        "new" | "/new" => Action::Reset,
        _ => Action::Ask(text),
    }
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Connects to Slack when set up (at start, and after the settings change).
pub fn start(app: &AppHandle) {
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Ok(config_dir) = crate::settings::get_config_dir(&app) {
            if !load(&config_dir).enabled {
                break;
            }
            let Some((app_token, bot_token)) = tokens().await else {
                break;
            };
            if let Err(e) = session(&app, &config_dir, &app_token, &bot_token).await {
                log::warn!("slack: {e}");
            }
            // Slack asks clients to reconnect from time to time; a failure waits a little first.
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// One Socket Mode connection, until Slack closes it.
async fn session(
    app: &AppHandle,
    config_dir: &Path,
    app_token: &str,
    bot_token: &str,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::Message as Ws;
    let open = call(app_token, "apps.connections.open", json!({})).await?;
    let url = open
        .get("url")
        .and_then(Value::as_str)
        .ok_or("Slack gave no connection address.")?;
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("could not connect: {e}"))?;
    let (mut tx, mut rx) = ws.split();
    while let Some(frame) = rx.next().await {
        let text = match frame.map_err(|e| e.to_string())? {
            Ws::Text(t) => t.to_string(),
            Ws::Ping(p) => {
                let _ = tx.send(Ws::Pong(p)).await;
                continue;
            }
            Ws::Close(_) => break,
            _ => continue,
        };
        let envelope: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        match envelope.get("type").and_then(Value::as_str) {
            Some("disconnect") => break,
            Some("events_api") => {}
            _ => continue,
        }
        // Acknowledge at once, or Slack sends the event again.
        if let Some(id) = envelope.get("envelope_id").and_then(Value::as_str) {
            tx.send(Ws::Text(json!({ "envelope_id": id }).to_string().into()))
                .await
                .map_err(|e| e.to_string())?;
        }
        let Some(event) = envelope.pointer("/payload/event").cloned() else {
            continue;
        };
        let channel = event
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // In a channel, answer in the thread of the mention; in a DM, plainly.
        let thread = if event.get("channel_type").and_then(Value::as_str) == Some("im") {
            None
        } else {
            event
                .get("thread_ts")
                .or_else(|| event.get("ts"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let mut settings = load(config_dir);
        let reply = match decide(&settings, &event) {
            Action::Ignore => continue,
            Action::Reply(t) => t,
            Action::Pair(user) => {
                if !settings.paired.contains(&user) {
                    settings.paired.push(user);
                }
                settings.pairing_code = None;
                let _ = save(config_dir, &settings);
                "Paired. Ask me anything.".into()
            }
            Action::Reset => {
                settings.chats.retain(|(c, _)| *c != channel);
                let _ = save(config_dir, &settings);
                "Started a fresh conversation.".into()
            }
            Action::Ask(question) => {
                let key = format!("{channel}:{}", thread.clone().unwrap_or_default());
                let existing = settings
                    .chats
                    .iter()
                    .find(|(c, _)| *c == key)
                    .map(|(_, id)| id.clone());
                let (id, answer) = super::telegram::run_message(
                    app,
                    config_dir,
                    settings.agent_id.clone(),
                    settings.allow_changes,
                    existing,
                    "Slack",
                    &question,
                )
                .await;
                if let Some(id) = id {
                    let mut s = load(config_dir);
                    s.chats.retain(|(c, _)| *c != key);
                    s.chats.push((key, id));
                    let _ = save(config_dir, &s);
                }
                answer.unwrap_or_else(|e| e)
            }
        };
        let mut body = json!({ "channel": channel, "text": reply });
        if let Some(ts) = &thread {
            body["thread_ts"] = json!(ts);
        }
        if let Err(e) = call(bot_token, "chat.postMessage", body).await {
            log::warn!("slack: could not reply: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_read_and_only_paired_members_answered() {
        let mut s = Settings {
            pairing_code: Some("123456".into()),
            ..Default::default()
        };
        let dm = |user: &str, text: &str| json!({"type": "message", "channel_type": "im", "user": user, "text": text, "channel": "D1", "ts": "1.0"});
        assert!(
            matches!(decide(&s, &dm("U1", "hi")), Action::Reply(t) if t.contains("only my owner"))
        );
        assert_eq!(
            decide(&s, &dm("U1", "pair 123456")),
            Action::Pair("U1".into())
        );
        s.paired.push("U1".into());
        assert_eq!(
            decide(&s, &dm("U1", "What's next?")),
            Action::Ask("What's next?".into())
        );
        assert_eq!(
            decide(
                &s,
                &json!({"type": "app_mention", "user": "U1", "text": "<@B9> summarize this", "channel": "C1", "ts": "2.0"})
            ),
            Action::Ask("summarize this".into())
        );
        // Bot messages, edits and channel chatter without a mention are not for it.
        assert_eq!(
            decide(
                &s,
                &json!({"type": "message", "channel_type": "im", "bot_id": "B9", "user": "U1", "text": "x"})
            ),
            Action::Ignore
        );
        assert_eq!(
            decide(
                &s,
                &json!({"type": "message", "channel_type": "im", "subtype": "message_changed", "user": "U1", "text": "x"})
            ),
            Action::Ignore
        );
        assert_eq!(
            decide(
                &s,
                &json!({"type": "message", "channel_type": "channel", "user": "U1", "text": "x"})
            ),
            Action::Ignore
        );
        assert_eq!(decide(&s, &dm("U1", "new")), Action::Reset);
        assert_eq!(clean("<@U0ABC> hello there "), "hello there");
    }
}
