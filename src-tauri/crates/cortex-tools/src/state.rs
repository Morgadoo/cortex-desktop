//! Which connectors are off, how each tool runs, and what the user approved: the state the
//! registry enforces. Loading and saving it per account is the app's (`tools::connector_state`).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    #[default]
    AutoApprove,
    Ask,
    Disable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectorState {
    #[serde(default)]
    pub disabled_connectors: HashSet<String>,
    /// connector_id -> tool_name -> policy
    #[serde(default)]
    pub tool_policies: HashMap<String, HashMap<String, ToolPolicy>>,
    /// connector_id -> tool_name -> [`fingerprint`](crate::registry::ToolDefinition::fingerprint)
    /// of an MCP tool as it was when the user chose to auto-approve it. The choice lapses back
    /// to asking when the server changes what it says the tool does.
    #[serde(default)]
    pub approved_tools: HashMap<String, HashMap<String, String>>,
    /// connector_id -> tool_name -> fingerprint (empty for built-in tools): tools the user lets
    /// run even after the conversation has read untrusted content, scheduled runs included.
    /// Like an approval, it lapses when an MCP server changes the tool.
    #[serde(default)]
    pub always_allowed: HashMap<String, HashMap<String, String>>,
    /// Connectors that have reached their remote at least once.
    ///
    /// Outlives the app, unlike the per-run activity: it separates a connector that is
    /// temporarily broken from one that has never functioned.
    #[serde(default)]
    pub succeeded_once: HashSet<String>,
    /// Conversations that have read private data or untrusted content, by conversation id.
    /// Kept so a conversation resumed after a restart is still gated; see `tools::exposure`.
    #[serde(default)]
    pub exposed_conversations: HashMap<String, crate::exposure::Exposure>,
}
