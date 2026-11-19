//! When a call may run: the approval modes, the calls that are hard to undo, and how a change
//! outside the connectors meets the mode. Pure decisions; asking the user is the host's.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool::ChatTool;

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

/// Calls that only read, though their tool is not declared read-only: browsing a page, and a GET
/// fetch.
pub fn reads_only(tool: &ChatTool, input: &Value) -> bool {
    match (tool.connector_id.as_deref(), tool.tool_name.as_str()) {
        (Some("browser"), "navigate") => true,
        (Some("computer"), _) => input.get("action").and_then(Value::as_str) == Some("screenshot"),
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
    if ["delete", "remove", "unlink", "rmdir"]
        .iter()
        .any(|w| name.contains(w))
    {
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
    RISKY
        .iter()
        .find(|(pattern, _)| squeezed.contains(pattern))
        .map(|(_, why)| *why)
}

/// The instruction for Auto mode's check: may this change run without asking?
pub fn auto_check_prompt(request: &str, tool: &str, input: &Value) -> String {
    let input = serde_json::to_string(input).unwrap_or_default();
    let input: String = input.chars().take(2_000).collect();
    format!(
        "You decide whether an AI assistant may take an action without asking its user first.\n\n\
         The user's latest request:\n{request}\n\nThe action: {tool} with input {input}\n\n\
         Reply with one line. Write ALLOW when the request clearly calls for this action and it is \
         easy to undo. Write ASK: and a short reason (under 15 words, addressed to the user) when \
         it goes beyond the request, touches things outside the user's project or folders, sends \
         or publishes anything to other people or services, spends money, deletes or overwrites \
         work, or is hard to undo."
    )
}

/// Reads the check's answer: `None` to go ahead, or the reason to ask. Anything unclear asks.
pub fn auto_check_verdict(answer: &str) -> Option<String> {
    let line = answer.trim().lines().next().unwrap_or_default().trim();
    if line.to_ascii_uppercase().starts_with("ALLOW") {
        return None;
    }
    let reason = line.split_once(':').map_or("", |(_, r)| r).trim();
    Some(if reason.is_empty() {
        "this goes beyond what you asked".into()
    } else {
        reason.trim_end_matches('.').to_string()
    })
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

/// How a change outside the connectors meets the approval mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeGate {
    Run,
    Ask,
    RefuseInPlan,
    RefuseUnattended,
}

/// A change outside the connectors asks in Ask and Accept edits, unless the user already
/// approved it (`allowed`: just now under an agent's policy, or "Always allow" in this chat).
/// Plan mode and scheduled runs nobody watches refuse it; Full auto and scheduled runs allowed to
/// make changes let it run.
pub fn change_gate(mode: ApprovalMode, allowed: bool) -> ChangeGate {
    match mode {
        ApprovalMode::Plan => ChangeGate::RefuseInPlan,
        ApprovalMode::Unattended => ChangeGate::RefuseUnattended,
        ApprovalMode::Ask | ApprovalMode::AcceptEdits if !allowed => ChangeGate::Ask,
        _ => ChangeGate::Run,
    }
}
