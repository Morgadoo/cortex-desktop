//! What a turn streams to the window: the [`ChatEvent`]s on `chat://event`, and how they are sent.
//!
//! In the bottom layer, so that anything that runs part of a turn (a coding agent, a recorder)
//! can report progress without depending on the engine.

use log::warn;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::chat::{ChatState, EVENT};

/// Where a sub-agent's run stands, as the page and the delegating model see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum RunStatus {
    Running,
    Done,
    Failed,
}

/// What the page sees of a turn, on [`crate::chat::EVENT`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub enum ChatEvent {
    #[serde(rename_all = "camelCase")]
    Started { conversation_id: String },
    /// The user's guidance, sent while the turn ran, has joined the conversation.
    #[serde(rename_all = "camelCase")]
    Steered {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Reasoning {
        conversation_id: String,
        text: String,
    },
    /// The model began writing a call to `name`; its arguments are still arriving.
    #[serde(rename_all = "camelCase")]
    ToolPending {
        conversation_id: String,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolStart {
        conversation_id: String,
        call_id: String,
        name: String,
        #[cfg_attr(feature = "typescript", ts(type = "Record<string, unknown>"))]
        arguments: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolEnd {
        conversation_id: String,
        call_id: String,
        output: String,
        is_error: bool,
    },
    /// The model asked the user something; answer with `chat_answer`.
    #[serde(rename_all = "camelCase")]
    Question {
        conversation_id: String,
        call_id: String,
        question: String,
        options: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Title {
        conversation_id: String,
        title: String,
    },
    /// A sub-agent of a `delegate` call started, moved on, or finished.
    #[serde(rename_all = "camelCase")]
    Subagent {
        conversation_id: String,
        call_id: String,
        index: usize,
        title: String,
        status: RunStatus,
        activity: String,
    },
    /// Tokens and cost so far, and how full the model's window is.
    #[serde(rename_all = "camelCase")]
    Usage {
        conversation_id: String,
        usage: cortex_kernel::context::UsageTotals,
        #[cfg_attr(feature = "typescript", ts(type = "number"))]
        context_window: u64,
    },
    /// Earlier turns are being summarized to make room.
    #[serde(rename_all = "camelCase")]
    Compacting { conversation_id: String },
    #[serde(rename_all = "camelCase")]
    Compacted {
        conversation_id: String,
        upto: usize,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        conversation_id: String,
        cancelled: bool,
    },
    /// Something the user should know, such as a switch to the backup model.
    #[serde(rename_all = "camelCase")]
    Notice {
        conversation_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        conversation_id: String,
        message: String,
    },
}

/// Sends `event` to the window, and keeps it for a window that opens while the turn runs.
pub(in crate::chat) fn emit(app: &AppHandle, event: ChatEvent) {
    if let (Some(state), Ok(value)) = (app.try_state::<ChatState>(), serde_json::to_value(&event)) {
        state.record(&value);
    }
    if let Err(e) = app.emit(EVENT, event) {
        warn!("could not emit chat event: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::RunStatus;

    #[test]
    fn run_statuses_keep_their_wire_names() {
        for (status, name) in [
            (RunStatus::Running, "running"),
            (RunStatus::Done, "done"),
            (RunStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), name);
        }
    }
}
