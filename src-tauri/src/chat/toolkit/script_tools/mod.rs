//! Tools the assistant writes for itself: a short JavaScript script with a name, a description and
//! an argument schema, run in the WebAssembly [`sandbox`].
//!
//! The lifecycle follows the kernel's rules. `create_tool` saves a *candidate* with no
//! capabilities and runs it once on test arguments; creating and testing is free. A candidate is
//! never offered to the model. `promote_tool` always asks the user, in every mode, showing the
//! script, the description the model will read and the capabilities asked for; once approved the
//! tool is *active*, its capabilities granted, and its script pinned by hash: a changed script is
//! refused until promoted again. Retired tools are *deprecated*: hidden, and restorable.
//!
//! Tools live in `chat/script-tools/<name>.json`, sealed at rest like the rest of the app's data.

pub mod sandbox;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::store::chat_dir;
use cortex_kernel::tool::ChatTool;
pub use sandbox::{Capabilities, Limits};

/// The connector id script tools are offered under (`script__<name>`).
pub const CONNECTOR: &str = "script";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Candidate,
    Active,
    Deprecated,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Candidate => "candidate",
            Status::Active => "active",
            Status::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub script: String,
    /// What the tool asked for when it was written.
    pub requested: Capabilities,
    /// What the user granted when promoting it; nothing for a candidate.
    #[serde(default)]
    pub granted: Capabilities,
    pub status: Status,
    /// The hash of the script the user approved; `None` until promoted.
    #[serde(default)]
    pub approved_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The conversation it was written in.
    #[serde(default)]
    pub provenance: Option<String>,
    /// Its regression checks: arguments and the result they must keep giving. A test run that
    /// worked becomes one when the tool is promoted.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// The latest test run, waiting to become a check.
    #[serde(default)]
    pub pending_check: Option<Check>,
    /// When the granted capabilities lapse; never when absent.
    #[serde(default)]
    pub granted_until: Option<DateTime<Utc>>,
    /// The app's signature over what the user approved, what the tool may reach and what the
    /// monitor and History go by (see `signed_part`), keyed by a secret in the OS keychain: a file
    /// changed outside the app no longer verifies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// When each granted capability lapses (`http:host`, `read:folder`, `write:folder`,
    /// `tool:name`, `cred:host=SECRET`); one without an entry lapses at `granted_until`, or never.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub expires: std::collections::BTreeMap<String, DateTime<Utc>>,
    /// When the monitor last rolled this tool back, so the same week's failures do not roll it
    /// back again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_reverted_at: Option<DateTime<Utc>>,
    /// When the user last approved it: the lifecycle and the monitor judge only calls since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// Versions (by approval hash) the user set aside by bringing back an earlier one from
    /// History: the monitor never rolls forward to them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_aside: Vec<String>,
    /// When the user last brought back an earlier version from History: until the version has a
    /// week of its own calls as a baseline, it is not compared week over week, since "last week"
    /// would be the versions the user set aside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<DateTime<Utc>>,
    /// How many times the app has signed this tool: each signed save is numbered above every one
    /// before it, so a later save whose record never reached the keychain (a crash, a failed
    /// write) is still told from an earlier one put back.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seq: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Each capability item with its key: `http:host`, `read:folder`, and so on.
fn keyed(c: &Capabilities) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, items) in [
        ("http", &c.http_hosts),
        ("read", &c.read_prefixes),
        ("write", &c.write_prefixes),
        ("tool", &c.tools),
        ("cred", &c.credentials),
    ] {
        out.extend(items.iter().map(|i| format!("{kind}:{i}")));
    }
    out
}

/// What a tool may reach at `now`: its grants, less each that has lapsed.
pub fn active_grants(tool: &ScriptTool, now: DateTime<Utc>) -> Capabilities {
    let live = |kind: &str, item: &String| {
        tool.expires
            .get(&format!("{kind}:{item}"))
            .copied()
            .or(tool.granted_until)
            .is_none_or(|until| until > now)
    };
    let keep = |kind: &str, items: &[String]| -> Vec<String> {
        items.iter().filter(|i| live(kind, i)).cloned().collect()
    };
    Capabilities {
        http_hosts: keep("http", &tool.granted.http_hosts),
        read_prefixes: keep("read", &tool.granted.read_prefixes),
        write_prefixes: keep("write", &tool.granted.write_prefixes),
        tools: keep("tool", &tool.granted.tools),
        credentials: keep("cred", &tool.granted.credentials),
    }
}

/// A regression check: these arguments must keep giving this result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub args: Value,
    pub expected: String,
}

fn dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("script-tools")
}

/// Lowercase letters, digits and underscores, 3 to 40 of them, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    (3..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn path(config_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(
            "A tool name uses lowercase letters, digits and underscores, like `count_words`."
                .into(),
        );
    }
    Ok(dir(config_dir).join(format!("{name}.json")))
}

/// The full SHA-256 of `content`, as hex: what an approval pins.
pub fn hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// What the user approves: the script, and the description and parameters the model reads to
/// decide when and how to call it. A change to any of them needs a new approval.
pub fn approval_hash(tool: &ScriptTool) -> String {
    hash(
        &serde_json::to_string(&json!({
            "script": tool.script,
            "description": tool.description,
            "parameters": tool.parameters,
        }))
        .unwrap_or_default(),
    )
}

pub fn load(config_dir: &Path, name: &str) -> Option<ScriptTool> {
    let text = crate::chat::at_rest::read_to_string(path(config_dir, name).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn list(config_dir: &Path) -> Vec<ScriptTool> {
    let mut out: Vec<ScriptTool> = std::fs::read_dir(dir(config_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let text = crate::chat::at_rest::read_to_string(e.path()).ok()?;
            serde_json::from_str(&text).ok()
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Saves a change to a tool, re-signed. A tool whose file was changed outside the app is not
/// changed further this way: its grants would be signed as if the app had made them. Only paths
/// that show the user the tool again (promotion) or take its access away use [`save_resigned`].
pub(crate) fn save(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    if let Some(on_disk) = load(config_dir, &tool.name) {
        if !verifies(config_dir, &on_disk) {
            return Err(format!(
                "`{}`'s file was changed outside the app, so it was not changed further; promote it again so the user sees what it is now.",
                tool.name
            ));
        }
    }
    save_resigned(config_dir, tool)
}

fn save_resigned(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    let path = path(config_dir, &tool.name)?;
    // Signed as saved by the app: what the user approved, and what it may reach, numbered above
    // every save before it.
    let previous = load(config_dir, &tool.name);
    let mut tool = tool.clone();
    // Numbered from what the app recorded or signed, never from a number written outside it.
    tool.seq = recorded_seq(config_dir, &tool.name)
        .max(
            previous
                .as_ref()
                .filter(|p| signed_by_app(config_dir, p))
                .map_or(0, |p| p.seq),
        )
        .saturating_add(1);
    tool.signature = sign(config_dir, &tool);
    let tool = &tool;
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    if let Some(previous) = previous {
        crate::chat::versions::keep(
            config_dir,
            "tools",
            &format!("script-{}", tool.name),
            &previous,
            tool,
        )?;
    }
    let text = serde_json::to_string_pretty(tool).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path, &text).map_err(|e| e.to_string())?;
    // The last state the app signed, kept apart from the file the tool runs from, and its
    // signature recorded as the latest.
    if let Some(signature) = &tool.signature {
        record_latest(config_dir, &tool.name, signature, tool.seq);
        let copy = signed_copy(config_dir, &tool.name)?;
        if let Some(parent) = copy.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::chat::at_rest::write(copy, &text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The tools on offer: the active ones, as the model sees them.
pub fn definitions(config_dir: &Path) -> Vec<ChatTool> {
    list(config_dir)
        .into_iter()
        // Only what the user approved and the app saved: a tool whose file was changed outside
        // the app is not offered, so its description never reaches the model.
        .filter(|t| {
            t.status == Status::Active
                && t.approved_hash.as_deref() == Some(approval_hash(t).as_str())
                && verifies(config_dir, t)
        })
        .map(|t| ChatTool {
            name: cortex_kernel::tool::safe_name(&format!("{CONNECTOR}__{}", t.name)),
            description: format!(
                "{} (A tool you wrote; it runs in a sandbox.)",
                t.description
            ),
            parameters: t.parameters.clone(),
            connector_id: Some(CONNECTOR.into()),
            tool_name: t.name.clone(),
            // Without network access or writes, its own or through the tools it calls, it can only
            // compute and read: nothing changes and nothing leaves the machine.
            read_only: !has_effects(config_dir, &t),
        })
        .collect()
}

/// Whether a tool can reach the network or write files, itself or through any tool it may call,
/// however deep: a tool's effect is the union of its callees' effects.
pub fn has_effects(config_dir: &Path, tool: &ScriptTool) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![tool.clone()];
    while let Some(tool) = queue.pop() {
        if !seen.insert(tool.name.clone()) {
            continue;
        }
        if !tool.granted.http_hosts.is_empty() || !tool.granted.write_prefixes.is_empty() {
            return true;
        }
        queue.extend(
            tool.granted
                .tools
                .iter()
                .filter_map(|callee| load(config_dir, callee)),
        );
    }
    false
}

mod authoring;
mod calling;
mod review;
mod signing;
mod tool_specs;
pub use authoring::*;
pub use calling::*;
pub use review::*;
pub use signing::*;
pub use tool_specs::*;

#[cfg(test)]
mod tests;
