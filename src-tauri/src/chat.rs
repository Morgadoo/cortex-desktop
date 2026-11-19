//! Native, multi-provider chat built on [rig](https://rig.rs).
//!
//! The main window is a bundled page (`webview/pages/app`), and every model call happens here in
//! Rust: the page sends a prompt with [`commands::chat_send`], and the reply streams back as
//! [`engine::ChatEvent`]s on the `chat://event` channel. Conversations are rig's own message
//! history, saved as JSON, so a reloaded conversation is exactly what the model saw.
//!
//! - [`providers`]: which providers are configured, their API keys (in the OS keychain) and models
//! - [`store`]: conversations, projects and the user's instructions, on disk
//! - [`engine`]: one turn: stream a reply, run the tools it calls, repeat
//! - [`tools`]: the app's connectors (filesystem, shell, MCP) and built-ins offered to the model

pub mod acp;
pub mod agent_cli;
pub mod agent_market;
pub mod agents;
pub mod animation;
pub mod api_server;
pub mod arena;
pub mod at_rest;
pub mod audio;
pub mod audit;
mod backup;
pub mod batched_qwen2;
pub mod builtins;
pub mod calendar;
pub mod canvas;
pub mod code;
pub mod coding_import;
pub mod commands;
pub mod computer;
pub mod config_cli;
pub mod context;
pub mod custom_tools;
mod deferred;
pub mod discord;
pub mod embeddings;
pub mod emoji_pack;
pub mod engine;
pub mod extraction;
mod favicon;
pub mod functions;
pub mod headless;
pub mod hooks;
pub mod import;
mod index;
pub mod knowledge;
pub mod learning;
pub mod libraries;
pub mod links;
pub mod local_models;
pub mod media;
pub mod memory;
pub mod openapi;
pub mod org;
pub mod outputs;
pub mod plugins;
mod preview;
pub mod privacy;
pub mod providers;
pub mod reader;
pub mod recent_files;
pub mod recorder;
pub mod router;
pub mod sandbox;
pub mod search;
pub mod search_all;
pub mod share;
pub mod sheets;
pub mod skill_catalog;
pub mod skills;
pub mod slack;
pub mod store;
pub mod subagents;
pub mod sync;
pub mod tasks;
pub mod telegram;
pub mod terminal;
pub mod terminal_chat;
pub mod tools;
pub mod ui_language;
pub mod uploads;
pub mod vault;
pub mod versions;
pub mod video;
pub mod videogen;
pub mod webpage;
pub mod workbench;
pub mod workflows;

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::{oneshot, watch};

/// The account every account-scoped subsystem (connectors, secrets, skills) runs under. Proton
/// accounts are gone; one local profile takes their place.
pub const LOCAL_ACCOUNT: &str = "local";

/// Event channel the page listens on.
pub const EVENT: &str = "chat://event";

/// Turns in flight and questions waiting for the user, shared by the commands.
#[derive(Default)]
pub struct ChatState {
    /// Conversation id → the sender that stops its running turn.
    running: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// `ask_user` call id → where the user's answer goes.
    questions: Mutex<HashMap<String, oneshot::Sender<String>>>,
    /// Conversation id → messages the user sent while its turn was running.
    steering: Mutex<HashMap<String, Vec<String>>>,
    /// Conversation id → what its running turn has streamed so far, so a window that opens (or
    /// reloads) during the turn shows it all, not only what comes next.
    replay: Mutex<HashMap<String, Vec<serde_json::Value>>>,
}

impl ChatState {
    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Registers a turn for `conversation`; `None` when one is already running.
    pub fn begin(&self, conversation: &str) -> Option<watch::Receiver<bool>> {
        let mut running = Self::lock(&self.running);
        if running.contains_key(conversation) {
            return None;
        }
        let (tx, rx) = watch::channel(false);
        running.insert(conversation.to_string(), tx);
        Some(rx)
    }

    pub fn finish(&self, conversation: &str) {
        Self::lock(&self.running).remove(conversation);
    }

    pub fn cancel(&self, conversation: &str) -> bool {
        match Self::lock(&self.running).get(conversation) {
            Some(tx) => tx.send(true).is_ok(),
            None => false,
        }
    }

    /// Queues guidance for a running turn; false when nothing is running.
    pub fn steer(&self, conversation: &str, text: String) -> bool {
        if !self.is_running(conversation) {
            return false;
        }
        Self::lock(&self.steering)
            .entry(conversation.to_string())
            .or_default()
            .push(text);
        true
    }

    /// The guidance waiting for a turn, removed from the queue.
    pub fn take_steering(&self, conversation: &str) -> Vec<String> {
        Self::lock(&self.steering)
            .remove(conversation)
            .unwrap_or_default()
    }

    pub fn has_steering(&self, conversation: &str) -> bool {
        Self::lock(&self.steering)
            .get(conversation)
            .is_some_and(|v| !v.is_empty())
    }

    pub fn is_running(&self, conversation: &str) -> bool {
        Self::lock(&self.running).contains_key(conversation)
    }

    /// Keeps a streamed event for replay: a new turn starts afresh, an ended one is forgotten,
    /// and text pieces in a row are joined so the buffer stays small.
    pub fn record(&self, event: &serde_json::Value) {
        let Some(id) = event
            .get("conversationId")
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        let kind = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let mut replay = Self::lock(&self.replay);
        match kind {
            "started" => {
                replay.insert(id.to_string(), vec![event.clone()]);
            }
            "done" | "error" => {
                replay.remove(id);
            }
            "title" | "usage" => {}
            _ => {
                let events = replay.entry(id.to_string()).or_default();
                if let (Some(last), Some(text)) = (
                    events.last_mut(),
                    event.get("text").and_then(serde_json::Value::as_str),
                ) {
                    if last.get("type").and_then(serde_json::Value::as_str) == Some(kind)
                        && matches!(kind, "text" | "reasoning")
                    {
                        let joined = format!(
                            "{}{text}",
                            last.get("text")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                        );
                        last["text"] = serde_json::Value::String(joined);
                        return;
                    }
                }
                if events.len() < 5_000 {
                    events.push(event.clone());
                }
            }
        }
    }

    /// What a running turn has streamed so far.
    pub fn replay(&self, conversation: &str) -> Vec<serde_json::Value> {
        if !self.is_running(conversation) {
            return Vec::new();
        }
        Self::lock(&self.replay)
            .get(conversation)
            .cloned()
            .unwrap_or_default()
    }

    pub fn ask(&self, call_id: &str) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        Self::lock(&self.questions).insert(call_id.to_string(), tx);
        rx
    }

    pub fn answer(&self, call_id: &str, answer: String) -> bool {
        match Self::lock(&self.questions).remove(call_id) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }

    pub fn forget_question(&self, call_id: &str) {
        Self::lock(&self.questions).remove(call_id);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_running_turn_can_be_replayed() {
        use serde_json::json;
        let state = ChatState::default();
        let _rx = state.begin("c").unwrap();
        state.record(&json!({"type": "started", "conversationId": "c"}));
        state.record(&json!({"type": "text", "conversationId": "c", "text": "Hel"}));
        state.record(&json!({"type": "text", "conversationId": "c", "text": "lo"}));
        state.record(&json!({"type": "toolCall", "conversationId": "c", "name": "web_search"}));
        state.record(&json!({"type": "text", "conversationId": "c", "text": "!"}));
        let r = state.replay("c");
        assert_eq!(r.len(), 4);
        assert_eq!(r[1]["text"], "Hello");
        state.record(&json!({"type": "done", "conversationId": "c"}));
        assert!(state.replay("c").is_empty());
        assert!(state.replay("other").is_empty());
    }

    #[test]
    fn guidance_waits_for_a_running_turn() {
        let state = ChatState::default();
        assert!(!state.steer("c", "ignored".into()), "nothing is running");
        let _rx = state.begin("c").unwrap();
        assert!(state.steer("c", "focus on the EU".into()));
        assert!(state.steer("c", "and keep it short".into()));
        assert!(state.has_steering("c"));
        assert_eq!(
            state.take_steering("c"),
            ["focus on the EU", "and keep it short"]
        );
        assert!(!state.has_steering("c"));
    }

    use super::*;

    #[test]
    fn one_turn_per_conversation() {
        let state = ChatState::default();
        let rx = state.begin("a").expect("first turn starts");
        assert!(state.begin("a").is_none());
        assert!(state.begin("b").is_some());
        assert!(state.cancel("a"));
        assert!(*rx.borrow());
        state.finish("a");
        assert!(!state.is_running("a"));
        assert!(state.begin("a").is_some());
    }

    #[test]
    fn answers_reach_the_waiting_question() {
        let state = ChatState::default();
        let mut rx = state.ask("call-1");
        assert!(state.answer("call-1", "yes".into()));
        assert_eq!(rx.try_recv().unwrap(), "yes");
        assert!(!state.answer("call-1", "again".into()));
    }
}
