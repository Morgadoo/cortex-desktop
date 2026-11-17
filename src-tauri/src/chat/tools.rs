//! The tools a conversation offers the model: the app's connectors, plus `ask_user`.
//!
//! A connector tool is named `{connector_id}__{tool}` as the web app named them, made safe for
//! every provider's name rules. Calls run through [`ToolRegistry::dispatch_with_transport`], so
//! connector readiness, per-tool policies and the untrusted-content gate all still apply. This
//! module adds the composer's approval mode on top: in "ask" mode every tool that can change
//! something waits for the user, and "accept edits" lets file edits through.

use std::time::Duration;

use rig_core::completion::request::ToolDefinition as RigTool;
use rig_core::completion::message::ToolName;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::tools::approvals;
use crate::tools::connector_state::ToolPolicy;
use crate::tools::registry::{ApprovalDecision, CallContext, ToolRegistry};

/// The built-in tool the model uses to ask the user a multiple-choice question.
pub const ASK_USER: &str = "ask_user";

/// The composer's approval setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Ask before any tool that can change something.
    #[default]
    Ask,
    /// File edits run without asking; other changes still ask.
    AcceptEdits,
    /// Plan first: the model investigates and proposes; anything that would change something is
    /// refused, with a note to describe the change instead.
    Plan,
    /// Full auto: nothing asks; the user is watching and can stop the turn.
    Auto,
    /// A scheduled run with nobody watching: anything that would ask is refused.
    Unattended,
    /// A scheduled run the user allowed to make changes: nothing asks.
    Autonomous,
}

impl ApprovalMode {
    /// Nobody is there to answer questions or approvals.
    pub fn unattended(self) -> bool {
        matches!(self, Self::Unattended | Self::Autonomous)
    }
}

/// A tool as offered to the model, and where calls to it go.
#[derive(Debug, Clone)]
pub struct ChatTool {
    /// The name the model sees.
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// `None` for `ask_user`.
    pub connector_id: Option<String>,
    pub tool_name: String,
    pub read_only: bool,
}

impl ChatTool {
    pub fn definition(&self) -> Option<RigTool> {
        Some(RigTool::new(
            ToolName::new(self.name.clone()).ok()?,
            self.description.clone(),
            self.parameters.clone(),
        ))
    }
}

/// Providers accept `^[a-zA-Z0-9_-]{1,64}$`; anything else becomes `_`.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    cleaned.chars().take(64).collect()
}

/// Every provider wants an object schema with properties, even for a tool without inputs.
fn normalize_schema(schema: &Value) -> Value {
    let mut schema = match schema {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let map = schema.as_object_mut().expect("object");
    map.remove("$schema");
    map.entry("type").or_insert_with(|| json!("object"));
    map.entry("properties").or_insert_with(|| json!({}));
    schema
}

fn ask_user_tool() -> ChatTool {
    ChatTool {
        name: ASK_USER.into(),
        description: "Ask the user a short clarifying question with 2 to 5 suggested answers, \
            when the request is ambiguous and a wrong guess would waste their time. The user can \
            also type their own answer. Do not use it for questions you can answer yourself."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "The question, in the user's language." },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "2 to 5 short suggested answers."
                }
            },
            "required": ["question", "options"]
        }),
        connector_id: None,
        tool_name: ASK_USER.into(),
        read_only: true,
    }
}

/// The tools of every enabled, ready connector, minus disabled ones, plus the built-ins.
/// `connectors`, when given, limits the offer to those connector ids.
pub fn available(registry: &ToolRegistry, connectors: Option<&[String]>) -> Vec<ChatTool> {
    let mut tools = vec![ask_user_tool()];
    for manifest in registry.all_manifests() {
        if !manifest.enabled || !manifest.readiness.is_ready() {
            continue;
        }
        if connectors.is_some_and(|ids| !ids.contains(&manifest.id)) {
            continue;
        }
        for tool in manifest.tools {
            if tool.policy == ToolPolicy::Disable {
                continue;
            }
            tools.push(ChatTool {
                name: safe_name(&format!("{}__{}", manifest.id, tool.name)),
                description: tool.description.clone(),
                parameters: normalize_schema(&tool.input_schema),
                connector_id: Some(manifest.id.clone()),
                tool_name: tool.name.clone(),
                read_only: tool.read_only == Some(true),
            });
        }
    }
    for tool in registry.builtin_tools() {
        let (connector, name) = tool
            .name
            .split_once("__")
            .map_or(("tool_results", tool.name.as_str()), |(c, n)| (c, n));
        tools.push(ChatTool {
            name: safe_name(&tool.name),
            description: tool.description.clone(),
            parameters: normalize_schema(&tool.input_schema),
            connector_id: Some(connector.to_string()),
            tool_name: name.to_string(),
            read_only: true,
        });
    }
    tools
}

/// Calls that only read, though their tool is not declared read-only: browsing a page, and a GET
/// fetch.
fn reads_only(tool: &ChatTool, input: &Value) -> bool {
    match (tool.connector_id.as_deref(), tool.tool_name.as_str()) {
        (Some("browser"), "navigate") => true,
        (Some("browser"), "fetch") => input
            .get("method")
            .and_then(Value::as_str)
            .is_none_or(|m| m.eq_ignore_ascii_case("GET")),
        _ => false,
    }
}

/// Commands and calls that are hard to undo, which ask even in Accept edits and Full auto, as
/// Vibe's smart-approve does. Each pattern comes with what the user is told.
const RISKY: &[(&str, &str)] = &[
    ("rm -rf", "it deletes files recursively"),
    ("rm -fr", "it deletes files recursively"),
    ("rm -r ", "it deletes folders"),
    ("sudo ", "it runs as administrator"),
    ("mkfs", "it formats a disk"),
    ("dd if=", "it writes raw data to a device"),
    ("> /dev/sd", "it writes to a disk device"),
    ("chmod -r 777", "it opens permissions on a whole tree"),
    ("git push --force", "it rewrites the remote history"),
    ("git push -f", "it rewrites the remote history"),
    ("git reset --hard", "it discards uncommitted work"),
    ("git clean -fd", "it deletes untracked files"),
    ("| sh", "it runs a downloaded script"),
    ("| bash", "it runs a downloaded script"),
    ("drop table", "it deletes a database table"),
    ("drop database", "it deletes a database"),
    ("shutdown", "it shuts the computer down"),
    ("reboot", "it restarts the computer"),
    (":(){", "it is a fork bomb"),
];

/// Why a call is risky, if it is: a dangerous command anywhere in its input, or a deletion.
pub fn risk(tool: &ChatTool, input: &Value) -> Option<&'static str> {
    if tool.read_only {
        return None;
    }
    let name = tool.tool_name.to_ascii_lowercase();
    if ["delete", "remove", "unlink", "rmdir"].iter().any(|w| name.contains(w)) {
        return Some("it deletes something");
    }
    fn texts(value: &Value, out: &mut String) {
        match value {
            Value::String(s) => {
                out.push_str(&s.to_ascii_lowercase());
                out.push('\n');
            }
            Value::Array(items) => items.iter().for_each(|v| texts(v, out)),
            Value::Object(map) => map.values().for_each(|v| texts(v, out)),
            _ => {}
        }
    }
    let mut text = String::new();
    texts(input, &mut text);
    let squeezed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    RISKY.iter().find(|(pattern, _)| squeezed.contains(pattern)).map(|(_, why)| *why)
}

/// Whether a call should wait for the user before it runs, under `mode`.
pub fn needs_approval(tool: &ChatTool, input: &Value, mode: ApprovalMode) -> bool {
    if tool.read_only || tool.connector_id.is_none() || reads_only(tool, input) {
        return false;
    }
    if risk(tool, input).is_some() {
        // Smart-approve: what is hard to undo always asks, whatever the mode.
        return true;
    }
    match mode {
        ApprovalMode::Ask | ApprovalMode::Unattended | ApprovalMode::Plan => true,
        ApprovalMode::Auto => false,
        ApprovalMode::AcceptEdits => tool.connector_id.as_deref() != Some("filesystem"),
        ApprovalMode::Autonomous => false,
    }
}

/// How long a call waits for the user to approve it.
const APPROVAL_WAIT: Duration = Duration::from_secs(15 * 60);

/// Asks the page to approve a call, with the payload the registry's own gate uses, so one
/// approval card serves both.
pub(crate) async fn request_approval(app: &AppHandle, tool: &ChatTool, input: &Value) -> ApprovalDecision {
    let reason = risk(tool, input).map(|why| json!({ "kind": "risky", "sources": [], "detail": why }));
    let (request_id, rx) = approvals::register();
    let _ = app.emit(
        "tool:approval_request",
        json!({
            "request_id": request_id,
            "connector_id": tool.connector_id,
            "tool_name": tool.tool_name,
            "input": input,
            "reason": reason,
        }),
    );
    match tokio::time::timeout(APPROVAL_WAIT, rx).await {
        Ok(Ok(decision)) => decision,
        _ => {
            approvals::cancel(&request_id);
            let _ = app.emit(
                "tool:approval_resolved",
                json!({ "request_id": request_id, "approved": false }),
            );
            ApprovalDecision::Deny
        }
    }
}

/// What a call returned, as text for the model, and whether it failed.
pub struct Outcome {
    pub text: String,
    pub is_error: bool,
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// Runs a connector tool call. `ask_user` is the engine's, not this function's.
pub async fn execute(
    app: &AppHandle,
    registry: &ToolRegistry,
    tool: &ChatTool,
    input: Value,
    mode: ApprovalMode,
    conversation_id: &str,
) -> Outcome {
    let Some(connector_id) = tool.connector_id.as_deref() else {
        return Outcome { text: "This tool is handled by the app.".into(), is_error: true };
    };
    if mode == ApprovalMode::Plan && needs_approval(tool, &input, mode) {
        return Outcome {
            text: "Plan mode: nothing is changed yet. Do not retry; finish your investigation and \
                present this step in your plan, so the user can approve the plan and switch modes."
                .into(),
            is_error: true,
        };
    }
    if mode == ApprovalMode::Unattended && needs_approval(tool, &input, mode) {
        return Outcome {
            text: "This scheduled run may not make changes, so the call was not made. Describe \
                what you would have done instead; the user can allow changes for this task."
                .into(),
            is_error: true,
        };
    }
    if mode == ApprovalMode::Autonomous {
        if let Some(why) = risk(tool, &input) {
            return Outcome {
                text: format!(
                    "This call was not made: {why}, and nobody is watching this scheduled run to \
                     approve it. Describe what you would have run instead."
                ),
                is_error: true,
            };
        }
    }
    let approved = if needs_approval(tool, &input, mode) {
        match request_approval(app, tool, &input).await {
            ApprovalDecision::Deny => {
                return Outcome {
                    text: "The user declined this tool call. Do not retry it; ask what they \
                        would like instead."
                        .into(),
                    is_error: true,
                }
            }
            ApprovalDecision::Once | ApprovalDecision::Always => true,
        }
    } else {
        matches!(mode, ApprovalMode::Autonomous | ApprovalMode::Auto)
    };
    let context = CallContext {
        conversation: Some(conversation_id),
        ..CallContext::default()
    };
    match registry
        .dispatch_with_transport(
            connector_id,
            &tool.tool_name,
            input,
            approved,
            false,
            false,
            context,
        )
        .await
    {
        Ok(value) => Outcome { text: value_text(&value), is_error: false },
        Err(e) => Outcome { text: e.to_ipc_string(), is_error: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(connector: Option<&str>, read_only: bool) -> ChatTool {
        ChatTool {
            name: "x".into(),
            description: String::new(),
            parameters: json!({}),
            connector_id: connector.map(Into::into),
            tool_name: "x".into(),
            read_only,
        }
    }

    #[test]
    fn names_are_made_safe_for_providers() {
        assert_eq!(safe_name("mcp-github__search.code"), "mcp-github__search_code");
        assert_eq!(safe_name(&"a".repeat(80)).len(), 64);
    }

    #[test]
    fn schemas_become_objects_with_properties() {
        let schema = normalize_schema(&json!({"$schema": "x"}));
        assert_eq!(schema, json!({"type": "object", "properties": {}}));
        let kept = normalize_schema(&json!({"type": "object", "properties": {"a": {}}}));
        assert_eq!(kept["properties"], json!({"a": {}}));
    }

    #[test]
    fn approval_follows_the_mode() {
        let none = json!({});
        let write = tool(Some("filesystem"), false);
        let shell = tool(Some("bash"), false);
        let read = tool(Some("filesystem"), true);
        assert!(needs_approval(&write, &none, ApprovalMode::Ask));
        // Smart-approve: risky calls ask even in full auto.
        let rm = json!({"command": "cd build &&  rm   -rf  ./out"});
        assert_eq!(risk(&shell, &rm), Some("it deletes files recursively"));
        assert!(needs_approval(&shell, &rm, ApprovalMode::Auto));
        assert!(needs_approval(&shell, &json!({"args": ["git", "push --force"]}), ApprovalMode::AcceptEdits));
        assert!(risk(&shell, &json!({"command": "ls -la && cargo test"})).is_none());
        assert!(!needs_approval(&shell, &json!({"command": "ls"}), ApprovalMode::Auto));
        assert!(needs_approval(&write, &none, ApprovalMode::Plan), "plan mode refuses what would ask");
        assert!(!needs_approval(&write, &none, ApprovalMode::Auto), "full auto asks for nothing");
        assert!(!needs_approval(&write, &none, ApprovalMode::AcceptEdits));
        assert!(needs_approval(&shell, &none, ApprovalMode::AcceptEdits));
        assert!(!needs_approval(&read, &none, ApprovalMode::Ask));
        assert!(!needs_approval(&ask_user_tool(), &none, ApprovalMode::Ask));
    }

    #[test]
    fn reading_the_web_needs_no_approval_but_posting_does() {
        let mut fetch = tool(Some("browser"), false);
        fetch.tool_name = "fetch".into();
        assert!(!needs_approval(&fetch, &json!({"url": "https://x"}), ApprovalMode::Ask));
        assert!(!needs_approval(&fetch, &json!({"method": "get"}), ApprovalMode::Ask));
        assert!(needs_approval(&fetch, &json!({"method": "POST"}), ApprovalMode::Ask));
        let mut navigate = tool(Some("browser"), false);
        navigate.tool_name = "navigate".into();
        assert!(!needs_approval(&navigate, &json!({}), ApprovalMode::Ask));
    }

    #[test]
    fn the_registry_offers_ask_user_even_without_connectors() {
        let registry = ToolRegistry::new();
        let tools = available(&registry, None);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, ASK_USER);
        assert!(tools[0].definition().is_some());
    }
}
