//! What the window is shown of a conversation: its parts, turns and versions.

use super::*;

/// One step of an assistant turn, as the page draws it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Part {
    Text {
        text: String,
    },
    /// Something the user should know about the reply, shown inline (Vibe 3.1.0): a backup
    /// model taking over, for example.
    Notice {
        text: String,
    },
    Reasoning {
        text: String,
        /// How long the model thought, in milliseconds, when measured.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "typescript", ts(optional))]
        ms: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        media_type: String,
        data: String,
    },
    #[serde(rename_all = "camelCase")]
    Tool {
        call_id: String,
        name: String,
        #[cfg_attr(feature = "typescript", ts(type = "Record<string, unknown>"))]
        arguments: Value,
        output: Option<String>,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Turn {
    /// `user` or `assistant`.
    #[cfg_attr(feature = "typescript", ts(type = "'user' | 'assistant'"))]
    pub role: &'static str,
    pub parts: Vec<Part>,
    /// Index of the turn's last message, which ratings and timestamps key on.
    pub index: usize,
    pub time: Option<DateTime<Utc>>,
    pub rating: i8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ConversationView {
    pub id: String,
    pub usage: cortex_kernel::context::UsageTotals,
    /// Index of the first turn requests still carry verbatim, when earlier ones were compacted.
    pub compacted_before: Option<usize>,
    pub title: String,
    pub project_id: Option<String>,
    pub workdir: Option<String>,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub params: ChatParams,
    /// Turn indexes the user bookmarked.
    pub bookmarks: Vec<usize>,
    pub turns: Vec<Turn>,
    /// The branch points along the showing path, each with its switcher position.
    pub versions: Vec<VersionsView>,
}

/// One point where a conversation branched: every continuation it had from there, the one
/// showing and the others in order around it. Regenerating an answer branches after its prompt;
/// editing a sent message branches at that message.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    /// Message index where the branches start.
    #[serde(alias = "prompt")]
    pub from: usize,
    /// `answer` (a regenerated reply) or `prompt` (an edited message). Empty for data saved before
    /// branching, whose branches start one message later.
    #[serde(default)]
    pub kind: String,
    /// The continuations not showing, in order, with the showing one at `current` between them.
    pub others: Vec<Vec<Message>>,
    /// Branch points inside each stored continuation, aligned with `others`.
    #[serde(default)]
    pub nested: Vec<Vec<Versions>>,
    pub current: usize,
}

impl Versions {
    /// Index of the first message the branches replace.
    pub(super) fn start(&self) -> usize {
        if self.kind.is_empty() {
            self.from + 1
        } else {
            self.from
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct VersionsView {
    /// Message index of the prompt the branches start at.
    pub from: usize,
    /// `answer` or `prompt`: which turn shows the switcher.
    #[cfg_attr(feature = "typescript", ts(type = "'answer' | 'prompt'"))]
    pub kind: String,
    /// 0-based position of the showing branch.
    pub current: usize,
    pub total: usize,
}
