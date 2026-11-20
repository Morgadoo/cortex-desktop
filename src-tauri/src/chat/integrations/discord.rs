//! Reach an agent from Discord (Onyx's and OpenBot's Discord bot): a bot of your own, connected
//! through Discord's Gateway, a connection Cortex opens, so nothing has to reach this computer from
//! outside.
//!
//! Setup: create an application at discord.com/developers, add a bot, copy its token, and invite
//! it to a server (OAuth2 → URL Generator, scope `bot`, permissions Send Messages and Read
//! Message History). Direct messages to the bot, and messages that mention it in a channel, are
//! answered there. Discord shows the text of those to a bot without the privileged Message
//! Content intent, so none is needed. Only Discord users paired with a one-time code are answered.

use crate::chat::platform;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

const TOKEN: &str = "discord-bot-token";
/// Discord's limit on a message's length.
const MAX_MESSAGE: usize = 2000;
/// Guild messages and direct messages; content comes with mentions and DMs regardless.
const INTENTS: u64 = (1 << 9) | (1 << 12);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub agent_id: Option<String>,
    /// Discord user ids allowed to talk to it.
    pub paired: Vec<String>,
    pub pairing_code: Option<String>,
    /// Discord channel id → Cortex conversation id.
    pub chats: Vec<(String, String)>,
    pub allow_changes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,
    pub has_token: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    crate::chat::store::chat_dir(config_dir).join("discord.json")
}

pub fn load(config_dir: &Path) -> Settings {
    crate::chat::store::read_json(&path(config_dir)).unwrap_or_default()
}

pub fn save(config_dir: &Path, s: &Settings) -> Result<(), String> {
    crate::chat::store::write_json(&path(config_dir), s)
}

/// Discord's REST API; `CORTEX_DISCORD_API` points elsewhere for testing.
fn api_base() -> String {
    std::env::var("CORTEX_DISCORD_API").unwrap_or_else(|_| "https://discord.com/api/v10".into())
}

pub async fn token() -> Option<String> {
    crate::chat::providers::api_key(TOKEN)
        .await
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())
}

pub async fn set_token(token: &str) -> Result<(), String> {
    let secrets = crate::chat::secrets::local()?;
    if token.trim().is_empty() {
        secrets
            .delete(&crate::chat::providers::secret_name(TOKEN))
            .await
    } else {
        secrets
            .store(&crate::chat::providers::secret_name(TOKEN), token.trim())
            .await
    }
}

async fn call(
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let client = crate::support::http::client(Duration::from_secs(30))?;
    let mut request = client
        .request(method, format!("{}{path}", api_base()))
        .header("Authorization", format!("Bot {token}"))
        .header("User-Agent", "DiscordBot (https://proton.me, 1) Cortex");
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Discord did not answer: {e}"))?;
    let status = response.status();
    let v: Value = response.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 401 {
        return Err("Discord refused the token (Developer Portal → Bot → Reset Token).".into());
    }
    if !status.is_success() {
        return Err(format!(
            "Discord: {}",
            v.get("message")
                .and_then(Value::as_str)
                .unwrap_or(status.as_str())
        ));
    }
    Ok(v)
}

/// Checks the token; returns the bot's name.
pub async fn check(token: &str) -> Result<String, String> {
    let me = call(token, reqwest::Method::GET, "/users/@me", None).await?;
    if me.get("bot").and_then(Value::as_bool) != Some(true) {
        return Err("That is not a bot token (Developer Portal → your application → Bot).".into());
    }
    Ok(me
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("bot")
        .to_string())
}

/// A message's text without mentions of the bot.
pub fn clean(text: &str, bot_id: &str) -> String {
    text.replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .trim()
        .to_string()
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Reply(String),
    Pair(String),
    Ask(String),
    Reset,
    Ignore,
}

/// What a `MESSAGE_CREATE` means. Bots (including this one) are ignored; in a server, only
/// messages that mention the bot count.
pub fn decide(settings: &Settings, message: &Value, bot_id: &str) -> Action {
    let author = message.get("author").cloned().unwrap_or(Value::Null);
    let user = author
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if author.get("bot").and_then(Value::as_bool) == Some(true) || user.is_empty() || user == bot_id
    {
        return Action::Ignore;
    }
    let in_server = message.get("guild_id").is_some_and(|g| !g.is_null());
    let mentioned = message
        .get("mentions")
        .and_then(Value::as_array)
        .is_some_and(|m| {
            m.iter()
                .any(|u| u.get("id").and_then(Value::as_str) == Some(bot_id))
        });
    if in_server && !mentioned {
        return Action::Ignore;
    }
    let text = clean(
        message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        bot_id,
    );
    if text.is_empty() {
        return Action::Ignore;
    }
    match platform::gate(&text, "pair", settings.pairing_code.as_deref(), &settings.paired, &user) {
        platform::Gate::Pair => Action::Pair(user),
        platform::Gate::WrongCode => Action::Reply("That code is not right, or pairing is closed. Open Cortex → Settings → General → Discord to get a code.".into()),
        platform::Gate::Stranger => Action::Reply("I answer only my owner. If that is you, send `pair` followed by the code shown in Cortex (Settings → General → Discord).".into()),
        platform::Gate::Owner("new" | "/new") => Action::Reset,
        platform::Gate::Owner(text) => Action::Ask(text.to_string()),
    }
}

static RUNNING: platform::OneAtATime = platform::OneAtATime::new();

/// Connects to Discord when set up (at start, and after the settings change).
pub fn start(app: &AppHandle) {
    platform::keep_connected::<Discord>(app, &RUNNING);
}

struct Discord;

#[async_trait::async_trait]
impl platform::Connection for Discord {
    const NAME: &'static str = "discord";
    type Credentials = String;

    fn enabled(config_dir: &Path) -> bool {
        load(config_dir).enabled
    }

    async fn credentials() -> Option<Self::Credentials> {
        token().await
    }

    async fn session(
        app: &AppHandle,
        config_dir: &Path,
        token: &Self::Credentials,
    ) -> Result<(), String> {
        session(app, config_dir, token).await
    }
}

/// One Gateway connection, until Discord closes it or the bot is turned off.
async fn session(app: &AppHandle, config_dir: &Path, token: &str) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::Message as Ws;
    let gateway = call(token, reqwest::Method::GET, "/gateway/bot", None).await?;
    let url = gateway
        .get("url")
        .and_then(Value::as_str)
        .ok_or("Discord gave no Gateway address.")?;
    let (ws, _) = tokio_tungstenite::connect_async(format!("{url}/?v=10&encoding=json"))
        .await
        .map_err(|e| format!("could not connect: {e}"))?;
    let (mut tx, mut rx) = ws.split();
    let mut seq: Option<u64> = None;
    let mut bot_id = String::new();
    // Until Hello says otherwise.
    let mut beat = tokio::time::interval(Duration::from_secs(3600));
    beat.tick().await;
    let mut checks = tokio::time::interval(Duration::from_secs(30));
    loop {
        let frame = tokio::select! {
            frame = rx.next() => frame,
            _ = beat.tick() => {
                tx.send(Ws::Text(json!({ "op": 1, "d": seq }).to_string().into())).await.map_err(|e| e.to_string())?;
                continue;
            }
            _ = checks.tick() => {
                if !load(config_dir).enabled {
                    let _ = tx.send(Ws::Close(None)).await;
                    return Ok(());
                }
                continue;
            }
        };
        let Some(frame) = frame else { return Ok(()) };
        let text = match frame.map_err(|e| e.to_string())? {
            Ws::Text(t) => t.to_string(),
            Ws::Ping(p) => {
                let _ = tx.send(Ws::Pong(p)).await;
                continue;
            }
            Ws::Close(c) => {
                return Err(format!(
                    "Discord closed the connection{}",
                    c.map(|c| format!(": {} {}", u16::from(c.code), c.reason))
                        .unwrap_or_default()
                ))
            }
            _ => continue,
        };
        let payload: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(s) = payload.get("s").and_then(Value::as_u64) {
            seq = Some(s);
        }
        match payload.get("op").and_then(Value::as_u64) {
            // Hello: start the heartbeat and identify.
            Some(10) => {
                let every = payload
                    .pointer("/d/heartbeat_interval")
                    .and_then(Value::as_u64)
                    .unwrap_or(41_250);
                beat = tokio::time::interval(Duration::from_millis(every));
                beat.tick().await;
                let identify = json!({ "op": 2, "d": { "token": token, "intents": INTENTS, "properties": { "os": std::env::consts::OS, "browser": "Cortex", "device": "Cortex" } } });
                tx.send(Ws::Text(identify.to_string().into()))
                    .await
                    .map_err(|e| e.to_string())?;
            }
            // Heartbeat asked for now.
            Some(1) => tx
                .send(Ws::Text(json!({ "op": 1, "d": seq }).to_string().into()))
                .await
                .map_err(|e| e.to_string())?,
            // Reconnect, or the session is no longer valid: start over.
            Some(7) | Some(9) => return Ok(()),
            Some(0) => match payload.get("t").and_then(Value::as_str) {
                Some("READY") => {
                    bot_id = payload
                        .pointer("/d/user/id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    log::info!(
                        "discord: connected as {}",
                        payload
                            .pointer("/d/user/username")
                            .and_then(Value::as_str)
                            .unwrap_or("the bot")
                    );
                }
                Some("MESSAGE_CREATE") => {
                    let message = payload.get("d").cloned().unwrap_or(Value::Null);
                    // Answer on the side, so the heartbeat keeps going during a long turn.
                    let (app, config_dir, token, bot_id) = (
                        app.clone(),
                        config_dir.to_path_buf(),
                        token.to_string(),
                        bot_id.clone(),
                    );
                    tauri::async_runtime::spawn(async move {
                        handle(&app, &config_dir, &token, &bot_id, message).await
                    });
                }
                _ => {}
            },
            _ => {}
        }
    }
}

async fn handle(app: &AppHandle, config_dir: &Path, token: &str, bot_id: &str, message: Value) {
    let channel = message
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let message_id = message
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut settings = load(config_dir);
    let reply = match decide(&settings, &message, bot_id) {
        Action::Ignore => return,
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
            let _ = call(
                token,
                reqwest::Method::POST,
                &format!("/channels/{channel}/typing"),
                Some(json!({})),
            )
            .await;
            let existing = settings
                .chats
                .iter()
                .find(|(c, _)| *c == channel)
                .map(|(_, id)| id.clone());
            let (id, answer) = crate::chat::telegram::run_message(
                app,
                config_dir,
                settings.agent_id.clone(),
                settings.allow_changes,
                existing,
                "Discord",
                &question,
            )
            .await;
            if let Some(id) = id {
                let mut s = load(config_dir);
                s.chats.retain(|(c, _)| *c != channel);
                s.chats.push((channel.clone(), id));
                let _ = save(config_dir, &s);
            }
            answer.unwrap_or_else(|e| e)
        }
    };
    for (i, piece) in crate::chat::telegram::chunks_of(&reply, MAX_MESSAGE)
        .into_iter()
        .enumerate()
    {
        let mut body = json!({ "content": piece, "allowed_mentions": { "parse": [] } });
        if i == 0 && !message_id.is_empty() {
            body["message_reference"] =
                json!({ "message_id": message_id, "fail_if_not_exists": false });
        }
        if let Err(e) = call(
            token,
            reqwest::Method::POST,
            &format!("/channels/{channel}/messages"),
            Some(body),
        )
        .await
        {
            log::warn!("discord: could not reply: {e}");
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_read_and_only_paired_users_answered() {
        let mut s = Settings {
            pairing_code: Some("123456".into()),
            ..Default::default()
        };
        let dm = |user: &str, text: &str| json!({ "id": "m1", "channel_id": "D1", "author": { "id": user, "username": "ana" }, "content": text, "mentions": [] });
        assert!(
            matches!(decide(&s, &dm("U1", "hi"), "B9"), Action::Reply(t) if t.contains("only my owner"))
        );
        assert_eq!(
            decide(&s, &dm("U1", "pair 123456"), "B9"),
            Action::Pair("U1".into())
        );
        s.paired.push("U1".into());
        assert_eq!(
            decide(&s, &dm("U1", "What's next?"), "B9"),
            Action::Ask("What's next?".into())
        );
        let mention = json!({ "id": "m2", "channel_id": "C1", "guild_id": "G1", "author": { "id": "U1" }, "content": "<@B9> summarize this", "mentions": [{ "id": "B9" }] });
        assert_eq!(
            decide(&s, &mention, "B9"),
            Action::Ask("summarize this".into())
        );
        // Server chatter without a mention, bots and its own messages are not for it.
        assert_eq!(
            decide(
                &s,
                &json!({ "guild_id": "G1", "author": { "id": "U1" }, "content": "x", "mentions": [] }),
                "B9"
            ),
            Action::Ignore
        );
        assert_eq!(
            decide(
                &s,
                &json!({ "author": { "id": "U7", "bot": true }, "content": "x" }),
                "B9"
            ),
            Action::Ignore
        );
        assert_eq!(
            decide(
                &s,
                &json!({ "author": { "id": "B9" }, "content": "x" }),
                "B9"
            ),
            Action::Ignore
        );
        assert_eq!(decide(&s, &dm("U1", "new"), "B9"), Action::Reset);
        assert_eq!(clean("<@!B9> hello there ", "B9"), "hello there");
        assert!(
            crate::chat::telegram::chunks_of(&"a".repeat(4500), MAX_MESSAGE)
                .iter()
                .all(|c| c.chars().count() <= MAX_MESSAGE)
        );
    }
}
