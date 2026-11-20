//! A [`Host`] with no window: a turn's events are kept and printed, nothing can be approved, and
//! token and price limits stop the turn. Headless runs and eval cases run turns on it.

use crate::chat::error::ChatError;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::chat::engine::ChatEvent;
use crate::chat::host::Host;
use crate::chat::tools::ChatTool;
use crate::tools::registry::ApprovalDecision;

/// What a headless run needs from the engine's [`Host`]: no window, no connectors, nobody to
/// approve. It keeps the turn's events for the export and stops the turn at a limit.
pub(crate) struct HeadlessHost {
    pub(in crate::chat) config_dir: PathBuf,
    pub(in crate::chat) conversation: String,
    pub(in crate::chat) state: crate::chat::ChatState,
    pub(in crate::chat) registry: crate::tools::registry::ToolRegistry,
    pub(in crate::chat) events: Mutex<Vec<ChatEvent>>,
    pub(in crate::chat) max_tokens: Option<u64>,
    pub(in crate::chat) max_price: Option<f64>,
    /// Why the run was stopped before its answer, when a limit stopped it.
    pub(in crate::chat) stopped: Mutex<Option<String>>,
}

impl HeadlessHost {
    /// A host with no limits, for other command-line runs (evals).
    pub(crate) fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            conversation: String::new(),
            state: crate::chat::ChatState::default(),
            registry: crate::tools::registry::ToolRegistry::new(),
            events: Mutex::new(Vec::new()),
            max_tokens: None,
            max_price: None,
            stopped: Mutex::new(None),
        }
    }

    pub(in crate::chat) fn stop(&self, why: String) {
        let mut stopped = self.stopped.lock().unwrap_or_else(|p| p.into_inner());
        if stopped.is_none() {
            *stopped = Some(why);
            self.state.cancel(&self.conversation);
        }
    }
}

#[async_trait::async_trait]
impl Host for HeadlessHost {
    fn config_dir(&self) -> Result<PathBuf, ChatError> {
        Ok(self.config_dir.clone())
    }

    fn emit(&self, event: ChatEvent) {
        match &event {
            ChatEvent::ToolStart { name, .. } => eprintln!("cortex: used {name}"),
            ChatEvent::Usage { usage, .. } => {
                let tokens = usage.input_tokens + usage.output_tokens;
                if let Some(limit) = self.max_tokens.filter(|l| tokens > *l) {
                    self.stop(format!("the token limit of {limit} was reached"));
                }
                if let Some(limit) = self.max_price.filter(|l| usage.cost > *l) {
                    self.stop(format!("the price limit of ${limit} was reached"));
                }
            }
            _ => {}
        }
        self.events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(event);
    }

    fn chat_state(&self) -> &crate::chat::ChatState {
        &self.state
    }

    fn registry(&self) -> &crate::tools::registry::ToolRegistry {
        &self.registry
    }

    async fn connect_model(
        &self,
        reference: &str,
    ) -> Result<rig_core::DynModel<rig_core::operation::Completion>, ChatError> {
        crate::chat::engine::connect_model(&self.config_dir, reference)
            .await
            .map_err(String::from)
            .map_err(ChatError::Invalid)
    }

    async fn approve(
        &self,
        _tool: &ChatTool,
        _input: &serde_json::Value,
        _reason: Option<&str>,
    ) -> ApprovalDecision {
        // Nobody is there to answer; the run is unattended, so the engine refuses first.
        ApprovalDecision::Deny
    }

    fn notify(&self, title: &str, _body: &str) {
        eprintln!("cortex: {title}");
    }
}
