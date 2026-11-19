//! What a turn needs from whatever runs it: the app's window, a test, later the command line.
//!
//! The engine talks to the user and the machine only through a [`Host`]: where data lives, where
//! events go, which model a reference connects to, and how a call is approved. [`TauriHost`] is
//! the desktop app. A test supplies scripted models and records what the turn emitted, so a whole
//! turn runs without a window.
//!
//! Tools that still need the Tauri app itself (the browser, computer use, agents messaging
//! agents) reach it through [`Host::app`], and say they are not available where there is none.

use std::path::PathBuf;

use async_trait::async_trait;
use rig_core::operation::Completion;
use rig_core::DynModel;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::engine::ChatEvent;
use super::tools::ChatTool;
use super::ChatState;
use crate::tools::registry::{ApprovalDecision, ToolRegistry};

// async-trait's generated methods trip this lint on current clippy; the futures are awaited.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Host: Send + Sync {
    /// The app's data folder.
    fn config_dir(&self) -> Result<PathBuf, String>;
    /// Shows a turn's progress to the user.
    fn emit(&self, event: ChatEvent);
    /// Running turns, steering and open questions.
    fn chat_state(&self) -> &ChatState;
    /// The connectors and their gate.
    fn registry(&self) -> &ToolRegistry;
    /// The model a reference such as `anthropic/claude-haiku-5-5` names.
    async fn connect_model(&self, reference: &str) -> Result<DynModel<Completion>, String>;
    /// Asks the user whether a call may run. `reason` is shown on the card.
    async fn approve(
        &self,
        tool: &ChatTool,
        input: &Value,
        reason: Option<&str>,
    ) -> ApprovalDecision;
    /// A system notification.
    fn notify(&self, title: &str, body: &str);
    /// The desktop app, for what is not behind this trait yet.
    fn app(&self) -> Option<&AppHandle> {
        None
    }
}

/// The desktop app.
pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl Host for TauriHost {
    fn config_dir(&self) -> Result<PathBuf, String> {
        crate::settings::get_config_dir(&self.app)
    }

    fn emit(&self, event: ChatEvent) {
        super::engine::emit(&self.app, event);
    }

    fn chat_state(&self) -> &ChatState {
        self.app.state::<ChatState>().inner()
    }

    fn registry(&self) -> &ToolRegistry {
        self.app.state::<ToolRegistry>().inner()
    }

    async fn connect_model(&self, reference: &str) -> Result<DynModel<Completion>, String> {
        let dir = self.config_dir()?;
        super::engine::connect_model(&dir, reference).await
    }

    async fn approve(
        &self,
        tool: &ChatTool,
        input: &Value,
        reason: Option<&str>,
    ) -> ApprovalDecision {
        super::tools::request_approval_because(&self.app, tool, input, reason).await
    }

    fn notify(&self, title: &str, body: &str) {
        super::engine::notify(&self.app, title, body);
    }

    fn app(&self) -> Option<&AppHandle> {
        Some(&self.app)
    }
}

/// A host for tests: scripted models by reference, a fixed answer to every approval, and a record
/// of what the turn emitted and asked.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    pub struct TestHost {
        pub dir: tempfile::TempDir,
        pub state: ChatState,
        pub registry: ToolRegistry,
        pub models: Mutex<HashMap<String, DynModel<Completion>>>,
        pub events: Mutex<Vec<ChatEvent>>,
        pub approvals: Mutex<Vec<String>>,
        /// The reason shown with each approval, as the card carries it.
        pub reasons: Mutex<Vec<String>>,
        pub answer: Mutex<ApprovalDecision>,
    }

    impl TestHost {
        pub fn new() -> Self {
            Self {
                dir: tempfile::tempdir().expect("temp dir"),
                state: ChatState::default(),
                registry: ToolRegistry::new(),
                models: Mutex::new(HashMap::new()),
                events: Mutex::new(Vec::new()),
                approvals: Mutex::new(Vec::new()),
                reasons: Mutex::new(Vec::new()),
                answer: Mutex::new(ApprovalDecision::Deny),
            }
        }

        /// Serves `model` for `reference`.
        pub fn model(&self, reference: &str, model: DynModel<Completion>) {
            self.models
                .lock()
                .unwrap()
                .insert(reference.to_string(), model);
        }

        pub fn events(&self) -> Vec<ChatEvent> {
            self.events.lock().unwrap().clone()
        }

        /// The text the turn streamed, joined.
        pub fn streamed_text(&self) -> String {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    ChatEvent::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl Host for TestHost {
        fn config_dir(&self) -> Result<PathBuf, String> {
            Ok(self.dir.path().to_path_buf())
        }

        fn emit(&self, event: ChatEvent) {
            self.events.lock().unwrap().push(event);
        }

        fn chat_state(&self) -> &ChatState {
            &self.state
        }

        fn registry(&self) -> &ToolRegistry {
            &self.registry
        }

        async fn connect_model(&self, reference: &str) -> Result<DynModel<Completion>, String> {
            self.models
                .lock()
                .unwrap()
                .get(reference)
                .cloned()
                .ok_or_else(|| format!("no scripted model for {reference}"))
        }

        async fn approve(
            &self,
            tool: &ChatTool,
            _input: &Value,
            reason: Option<&str>,
        ) -> ApprovalDecision {
            self.approvals.lock().unwrap().push(tool.name.clone());
            self.reasons
                .lock()
                .unwrap()
                .push(reason.unwrap_or_default().to_string());
            *self.answer.lock().unwrap()
        }

        fn notify(&self, title: &str, _body: &str) {
            self.emit(ChatEvent::Notice {
                conversation_id: String::new(),
                text: format!("notification: {title}"),
            });
        }
    }
}
