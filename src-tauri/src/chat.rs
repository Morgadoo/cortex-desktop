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

pub mod agents;
pub mod builtins;
pub mod canvas;
pub mod code;
pub mod commands;
pub mod context;
pub mod engine;
pub mod libraries;
pub mod media;
pub mod memory;
pub mod privacy;
pub mod providers;
pub mod sandbox;
pub mod search;
pub mod skills;
pub mod store;
pub mod subagents;
pub mod tasks;
pub mod tools;
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
}

impl ChatState {
    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

    pub fn is_running(&self, conversation: &str) -> bool {
        Self::lock(&self.running).contains_key(conversation)
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
