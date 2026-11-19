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

use super::store::chat_dir;
use super::tools::ChatTool;
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

pub fn hash(script: &str) -> String {
    super::traces::hash(script)
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

fn save(config_dir: &Path, tool: &ScriptTool) -> Result<(), String> {
    let path = path(config_dir, &tool.name)?;
    std::fs::create_dir_all(dir(config_dir)).map_err(|e| e.to_string())?;
    if let Some(previous) = load(config_dir, &tool.name) {
        super::versions::keep(
            config_dir,
            "tools",
            &format!("script-{}", tool.name),
            &previous,
            tool,
        )?;
    }
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(tool).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// The tools on offer: the active ones, as the model sees them.
pub fn definitions(config_dir: &Path) -> Vec<ChatTool> {
    list(config_dir)
        .into_iter()
        .filter(|t| t.status == Status::Active)
        .map(|t| ChatTool {
            name: super::tools::safe_name(&format!("{CONNECTOR}__{}", t.name)),
            description: format!(
                "{} (A tool you wrote; it runs in a sandbox.)",
                t.description
            ),
            parameters: t.parameters.clone(),
            connector_id: Some(CONNECTOR.into()),
            tool_name: t.name.clone(),
            // Without network access it can only compute and read: nothing leaves the machine.
            read_only: t.granted.http_hosts.is_empty(),
        })
        .collect()
}

/// What `create_tool` asks for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Draft {
    name: String,
    description: String,
    #[serde(default)]
    parameters: Option<Value>,
    script: String,
    #[serde(default)]
    test_args: Option<Value>,
    #[serde(default)]
    http_hosts: Vec<String>,
    #[serde(default)]
    read_prefixes: Vec<String>,
    /// Other approved tools it calls with `tool(name, args)`.
    #[serde(default)]
    tools: Vec<String>,
    /// Save it even though a similar tool exists.
    #[serde(default)]
    new_anyway: bool,
}

/// An existing tool that does what `description` says, if one does: the kernel's rule is to
/// search before creating and to update a tool rather than add a near-duplicate.
fn near_duplicate<'a>(
    tools: &'a [ScriptTool],
    name: &str,
    description: &str,
) -> Option<&'a ScriptTool> {
    use std::collections::HashSet;
    let words = |t: &str| -> HashSet<String> {
        super::retrieval::tokens(t)
            .into_iter()
            .filter(|w| w.chars().count() > 3 && !matches!(w.as_str(), "tool" | "tools"))
            .collect()
    };
    let new = words(&format!("{} {description}", name.replace('_', " ")));
    if new.is_empty() {
        return None;
    }
    tools
        .iter()
        .filter(|t| t.name != name && t.status != Status::Deprecated)
        .map(|t| {
            let old = words(&format!("{} {}", t.name.replace('_', " "), t.description));
            let shared = new.intersection(&old).count() as f64;
            let union = new.union(&old).count().max(1) as f64;
            (t, shared / union)
        })
        .filter(|(_, overlap)| *overlap >= 0.5)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t)
}

/// Runs `script` on each check; the failures, in words.
async fn failing_checks(script: &str, checks: &[Check]) -> Vec<String> {
    let mut failed = Vec::new();
    for check in checks {
        let (result, _) = sandbox::run(
            script,
            &check.args.to_string(),
            &Capabilities::default(),
            Limits::default(),
            None,
        )
        .await;
        match result {
            Ok(out) if out == check.expected => {}
            Ok(out) => failed.push(format!(
                "{} gave {} instead of {}",
                check.args,
                out.chars().take(200).collect::<String>(),
                check.expected.chars().take(200).collect::<String>()
            )),
            Err(e) => failed.push(format!("{} failed: {e}", check.args)),
        }
    }
    failed
}

/// Names a written tool may not take: the app's own tools.
fn taken(name: &str) -> bool {
    super::builtins::NAMES.contains(&name)
        || [
            super::builtins::DELEGATE,
            super::builtins::CANVAS,
            super::builtins::NOTIFY,
            CREATE_TOOL,
            PROMOTE_TOOL,
            RETIRE_TOOL,
            REQUEST_CAPABILITY,
        ]
        .contains(&name)
}

pub const CREATE_TOOL: &str = "create_tool";
pub const PROMOTE_TOOL: &str = "promote_tool";
pub const RETIRE_TOOL: &str = "retire_tool";
pub const REQUEST_CAPABILITY: &str = "request_capability";

/// `create_tool`: saves (or replaces) a candidate and runs it once on its test arguments, with no
/// capabilities. A promoted tool that is rewritten goes back to being a candidate.
pub async fn create(config_dir: &Path, conversation: &str, args: &Value) -> Result<String, String> {
    let draft: Draft = serde_json::from_value(args.clone())
        .map_err(|e| format!("The tool is not described right: {e}"))?;
    let name = draft.name.trim().to_string();
    if taken(&name) {
        return Err(format!(
            "`{name}` is one of the app's own tools; choose another name."
        ));
    }
    if draft.description.trim().is_empty() {
        return Err("Describe what the tool does and when to use it.".into());
    }
    if !draft.script.contains("function main") {
        return Err("The script must define `function main(args)` and return the result.".into());
    }
    let parameters = draft
        .parameters
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    if !parameters.is_object() {
        return Err("`parameters` must be a JSON schema object.".into());
    }
    let existing = list(config_dir);
    if !draft.new_anyway && !existing.iter().any(|t| t.name == name) {
        if let Some(similar) = near_duplicate(&existing, &name, &draft.description) {
            return Err(format!(
                "`{}` already looks like this tool ({}). Update it by calling create_tool with name \
                 `{}`, or, if this is really a different tool, call again with new_anyway: true.",
                similar.name, similar.description, similar.name
            ));
        }
    }
    let previous = existing.into_iter().find(|t| t.name == name);
    let checks = previous
        .as_ref()
        .map(|t| t.checks.clone())
        .unwrap_or_default();
    let tool = ScriptTool {
        name: name.clone(),
        description: draft.description.trim().to_string(),
        parameters,
        script: draft.script,
        requested: Capabilities {
            http_hosts: draft.http_hosts,
            read_prefixes: draft.read_prefixes,
            tools: draft.tools,
        },
        granted: Capabilities::default(),
        status: Status::Candidate,
        approved_hash: None,
        created_at: Utc::now(),
        provenance: (!conversation.is_empty()).then(|| conversation.to_string()),
        checks,
        pending_check: None,
        granted_until: None,
    };
    let mut tool = tool;
    save(config_dir, &tool)?;
    super::traces::register(
        config_dir,
        "script_tool",
        &name,
        Some(&hash(&tool.script)),
        tool.provenance.as_deref(),
        Status::Candidate.as_str(),
    );
    // The test run: no capabilities, whatever the tool asked for.
    let test_args = draft.test_args.unwrap_or_else(|| json!({})).to_string();
    let (result, logs) = sandbox::run(
        &tool.script,
        &test_args,
        &Capabilities::default(),
        Limits::default(),
        None,
    )
    .await;
    let test = match result {
        Ok(output) => {
            tool.pending_check = Some(Check {
                args: serde_json::from_str(&test_args).unwrap_or(Value::Null),
                expected: output.clone(),
            });
            save(config_dir, &tool)?;
            json!({ "ok": true, "output": output.chars().take(4_000).collect::<String>() })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    };
    // A rewrite must keep passing the checks the tool already had.
    let regressions = failing_checks(&tool.script, &tool.checks).await;
    Ok(json!({
        "saved": name,
        "status": "candidate",
        "test": test,
        "checks": { "count": tool.checks.len(), "failing": regressions },
        "logs": logs.len(),
        "next": "Fix the script with create_tool if the test is wrong. When it works, call promote_tool so the user can review and approve it; until then it is not offered. Its test ran with no network or file access, whatever it asked for."
    })
    .to_string())
}

/// What the approval card shows for a promotion: the script, the description the model will read
/// and the capabilities asked for.
pub fn review(tool: &ScriptTool) -> Value {
    json!({
        "tool": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "access": if tool.requested.is_empty() {
            "none: it can only compute"
        } else {
            "the network hosts and folders below"
        },
        "network": tool.requested.http_hosts,
        "reads": tool.requested.read_prefixes,
        "calls": tool.requested.tools,
        "script": tool.script,
    })
}

/// Makes an approved candidate active: its regression checks must pass, then its capabilities are
/// granted (for `days`, or until revoked) and its script pinned. Its last good test run becomes a
/// check.
pub async fn promote(config_dir: &Path, name: &str, days: Option<i64>) -> Result<String, String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    let failing = failing_checks(&tool.script, &tool.checks).await;
    if !failing.is_empty() {
        return Err(format!(
            "`{name}` fails its regression checks, so it was not promoted: {}",
            failing.join("; ")
        ));
    }
    if let Some(check) = tool.pending_check.take() {
        if !tool.checks.iter().any(|c| c.args == check.args) {
            tool.checks.push(check);
        }
    }
    tool.granted_until = days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + chrono::Duration::days(d));
    tool.granted = tool.requested.clone();
    tool.status = Status::Active;
    tool.approved_hash = Some(hash(&tool.script));
    save(config_dir, &tool)?;
    super::traces::register(
        config_dir,
        "script_tool",
        name,
        tool.approved_hash.as_deref(),
        None,
        Status::Active.as_str(),
    );
    Ok(format!(
        "`{name}` is now a tool, available in every chat as script__{name}."
    ))
}

/// Takes a tool off offer until it is promoted again: the lifecycle does this to one that keeps
/// failing.
pub fn demote(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.status = Status::Candidate;
    tool.granted = Capabilities::default();
    save(config_dir, &tool)?;
    super::traces::set_status(config_dir, "script_tool", name, Status::Candidate.as_str());
    Ok(())
}

/// Takes back a tool's capabilities; it stays on offer, able only to compute.
pub fn revoke(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.granted = Capabilities::default();
    tool.requested = Capabilities::default();
    tool.granted_until = None;
    save(config_dir, &tool)
}

/// Sets what a tool asks for, before the user is asked to grant it.
pub fn request(config_dir: &Path, name: &str, wanted: Capabilities) -> Result<ScriptTool, String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.requested = wanted;
    save(config_dir, &tool)?;
    Ok(tool)
}

/// Grants a tool what it asked for, for `days` or until revoked.
pub fn grant(config_dir: &Path, name: &str, days: Option<i64>) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.granted = tool.requested.clone();
    tool.granted_until = days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + chrono::Duration::days(d));
    save(config_dir, &tool)
}

/// Runs every active tool's regression checks; a tool that fails one goes back to candidate.
/// Returns the tools demoted.
pub async fn check_all(config_dir: &Path) -> Vec<String> {
    let mut demoted = Vec::new();
    for tool in list(config_dir)
        .into_iter()
        .filter(|t| t.status == Status::Active && !t.checks.is_empty())
    {
        if !failing_checks(&tool.script, &tool.checks).await.is_empty()
            && demote(config_dir, &tool.name).is_ok()
        {
            demoted.push(tool.name);
        }
    }
    demoted
}

/// Calls per tool per minute; past it, calls are refused until the minute passes.
const CALLS_PER_MINUTE: usize = 30;

fn within_rate(name: &str) -> bool {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static CALLS: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();
    let mut calls = CALLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let recent = calls.entry(name.to_string()).or_default();
    let now = Instant::now();
    while recent
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
    {
        recent.pop_front();
    }
    if recent.len() >= CALLS_PER_MINUTE {
        return false;
    }
    recent.push_back(now);
    true
}

/// Hides a tool; promoting it again brings it back.
pub fn retire(config_dir: &Path, name: &str) -> Result<(), String> {
    let mut tool = load(config_dir, name).ok_or("There is no tool by that name.")?;
    tool.status = Status::Deprecated;
    save(config_dir, &tool)?;
    super::traces::set_status(config_dir, "script_tool", name, Status::Deprecated.as_str());
    Ok(())
}

/// How deep tools may call tools: a script calling a tool that calls a tool, and no further.
const MAX_DEPTH: usize = 3;

/// Runs an active tool with its granted capabilities, after checking its script is the one the
/// user approved. `workspace` is the chat's folder for `read`. A script may call the other tools
/// its capabilities name, with the same checks, a few levels deep.
pub async fn call(
    config_dir: &Path,
    name: &str,
    args: &Value,
    workspace: Option<&Path>,
) -> Result<String, String> {
    call_at(
        config_dir.to_path_buf(),
        name.to_string(),
        args.clone(),
        workspace.map(Path::to_path_buf),
        0,
    )
    .await
}

fn call_at(
    config_dir: PathBuf,
    name: String,
    args: Value,
    workspace: Option<PathBuf>,
    depth: usize,
) -> futures::future::BoxFuture<'static, Result<String, String>> {
    Box::pin(async move {
        if depth >= MAX_DEPTH {
            return Err(format!(
                "`{name}` is called too deep; tools call tools {MAX_DEPTH} levels at most."
            ));
        }
        let tool = load(&config_dir, &name).ok_or("That tool was removed.")?;
        if tool.status != Status::Active {
            return Err("That tool is not active; promote it first.".into());
        }
        if tool.approved_hash.as_deref() != Some(hash(&tool.script).as_str()) {
            return Err(
                "This tool's script changed since it was approved; promote it again.".into(),
            );
        }
        if !within_rate(&name) {
            return Err(format!(
                "`{name}` was called {CALLS_PER_MINUTE} times in the last minute; wait before calling it again."
            ));
        }
        // Capabilities granted for a time lapse; the tool can still compute.
        let granted = if tool.granted_until.is_some_and(|until| until < Utc::now()) {
            Capabilities::default()
        } else {
            tool.granted.clone()
        };
        let caller: sandbox::ToolCaller = {
            let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
            std::sync::Arc::new(move |callee: String, input: String| {
                let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
                Box::pin(async move {
                    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
                    call_at(config_dir, callee, input, workspace, depth + 1).await
                })
            })
        };
        let workspace = workspace.map(|root| sandbox::Workspace {
            root,
            // Files the app sealed at rest are opened with its key.
            read: |p| crate::chat::at_rest::read(p),
        });
        let (result, logs) = sandbox::run_with_tools(
            &tool.script,
            &args.to_string(),
            &granted,
            Limits::default(),
            workspace,
            Some(caller),
        )
        .await;
        // The tool's own lines are untrusted text: they go to the log file, never to the model.
        for line in logs {
            log::info!("[script tool {name}] {line}");
        }
        result.map_err(|e| e.to_string())
    })
}

/// The definitions of `create_tool` and `promote_tool`.
pub fn builtin_definitions() -> Vec<ChatTool> {
    let tool = |name: &str, description: &str, parameters: Value, read_only: bool| ChatTool {
        name: name.into(),
        description: description.into(),
        parameters,
        connector_id: None,
        tool_name: name.into(),
        read_only,
    };
    vec![
        tool(
            CREATE_TOOL,
            "Write a reusable tool for yourself when a task needs a small, exact computation or \
             fetch you would otherwise repeat: a JavaScript script that defines `function \
             main(args)` and returns a JSON value. It runs in a sandbox with no network or files \
             unless you ask for them: `fetch(method, url, body)` (https, listed hosts only) returns \
             {status, body}; `read(path)` reads a file in the chat's folder; `log(text)` notes \
             progress. It is saved as a candidate and tested once on `test_args`; call \
             promote_tool once it works, and the user decides. `tool(name, args)` calls another tool \
             you wrote, when you list it in `tools`.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "lowercase_with_underscores" },
                    "description": { "type": "string", "description": "What it does and when to use it, for your future self." },
                    "parameters": { "type": "object", "description": "JSON schema of the arguments." },
                    "script": { "type": "string", "description": "JavaScript defining function main(args)." },
                    "test_args": { "type": "object", "description": "Arguments for the test run." },
                    "http_hosts": { "type": "array", "items": { "type": "string" }, "description": "Exact host names it needs to reach over https, if any." },
                    "read_prefixes": { "type": "array", "items": { "type": "string" }, "description": "Folders of the chat's workspace it needs to read, if any." },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Other tools you wrote that it calls with tool(name, args), so their results never pass through the chat." },
                    "new_anyway": { "type": "boolean", "description": "Save it although a similar tool exists (you were told which); prefer updating that one." }
                },
                "required": ["name", "description", "script"]
            }),
            false,
        ),
        tool(
            PROMOTE_TOOL,
            "Ask the user to make a tool you wrote with create_tool available in every chat. They \
             see its script, description and access, and approve or decline.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "days": { "type": "integer", "minimum": 1, "description": "Grant its network and file access for this many days only." }
                },
                "required": ["name"]
            }),
            false,
        ),
        tool(
            RETIRE_TOOL,
            "Retire a tool you wrote that is no longer useful or was replaced; it stops being \
             offered and can be promoted again later.",
            json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            false,
        ),
        tool(
            REQUEST_CAPABILITY,
            "Ask the user to let one of your tools reach more: https hosts or folders of the chat's \
             workspace, for a number of days, with the reason. Nothing changes until they approve.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "http_hosts": { "type": "array", "items": { "type": "string" } },
                    "read_prefixes": { "type": "array", "items": { "type": "string" } },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "days": { "type": "integer", "minimum": 1 },
                    "reason": { "type": "string" }
                },
                "required": ["name", "reason"]
            }),
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str =
        "function main(a) { return { words: a.text.split(/\\s+/).filter(Boolean).length }; }";

    #[tokio::test]
    async fn a_tool_is_written_tested_promoted_and_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let out = create(
            d,
            "chat-1",
            &json!({
                "name": "count_words",
                "description": "Count the words in a text.",
                "parameters": { "type": "object", "properties": { "text": { "type": "string" } } },
                "script": SCRIPT,
                "test_args": { "text": "one two  three" }
            }),
        )
        .await
        .unwrap();
        assert!(out.contains(r#"\"words\":3"#), "{out}");
        // A candidate is not offered.
        assert!(definitions(d).is_empty());
        assert!(call(d, "count_words", &json!({"text": "a"}), None)
            .await
            .is_err());

        promote(d, "count_words", None).await.unwrap();
        let offered = definitions(d);
        assert_eq!(offered[0].name, "script__count_words");
        assert!(offered[0].read_only);
        assert_eq!(
            call(d, "count_words", &json!({"text": "a b"}), None)
                .await
                .unwrap(),
            r#"{"words":2}"#
        );
        assert_eq!(
            super::super::traces::artifact(d, "script_tool", "count_words").map(|a| a.status),
            Some("active".into())
        );

        // A script changed behind the approval is refused.
        let mut tampered = load(d, "count_words").unwrap();
        tampered.script = "function main() { return 'changed'; }".into();
        crate::chat::at_rest::write(
            path(d, "count_words").unwrap(),
            serde_json::to_string(&tampered).unwrap(),
        )
        .unwrap();
        let refused = call(d, "count_words", &json!({}), None).await.unwrap_err();
        assert!(
            refused.contains("changed since it was approved"),
            "{refused}"
        );

        retire(d, "count_words").unwrap();
        assert!(definitions(d).is_empty());
    }

    /// Search before create: a tool that does the same thing is updated, not duplicated.
    #[tokio::test]
    async fn a_near_duplicate_is_refused_unless_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let draft = |name: &str, anyway: bool| {
            json!({
                "name": name,
                "description": "Count the words in a text, ignoring extra spaces.",
                "script": SCRIPT,
                "new_anyway": anyway
            })
        };
        create(d, "", &draft("count_words", false)).await.unwrap();
        let refused = create(d, "", &draft("word_counter", false))
            .await
            .unwrap_err();
        assert!(refused.contains("count_words"), "{refused}");
        // Updating the same tool is fine, and so is insisting.
        create(d, "", &draft("count_words", false)).await.unwrap();
        create(d, "", &draft("word_counter", true)).await.unwrap();
    }

    /// A promoted tool keeps its good test run as a check; a rewrite that breaks it is not promoted.
    #[tokio::test]
    async fn regression_checks_hold_a_tool_to_what_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let draft = |script: &str| {
            json!({
                "name": "count_words",
                "description": "Count words.",
                "script": script,
                "test_args": { "text": "a b c" }
            })
        };
        create(d, "", &draft(SCRIPT)).await.unwrap();
        promote(d, "count_words", None).await.unwrap();
        assert_eq!(load(d, "count_words").unwrap().checks.len(), 1);
        let broken = "function main(a) { return { words: 0 }; }";
        let out = create(d, "", &draft(broken)).await.unwrap();
        assert!(out.contains("instead of"), "{out}");
        let refused = promote(d, "count_words", None).await.unwrap_err();
        assert!(refused.contains("regression checks"), "{refused}");
        // The daily pass demotes an active tool whose checks fail.
        create(d, "", &draft(SCRIPT)).await.unwrap();
        promote(d, "count_words", None).await.unwrap();
        let mut tampered = load(d, "count_words").unwrap();
        tampered.checks[0].expected = "{\"words\":99}".into();
        save(d, &tampered).unwrap();
        assert_eq!(check_all(d).await, vec!["count_words".to_string()]);
    }

    /// Access granted for a time lapses; revoking takes it back at once.
    #[tokio::test]
    async fn granted_access_lapses_and_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        create(
            d,
            "",
            &json!({
                "name": "fetcher",
                "description": "Fetch a page.",
                "script": "function main() { return fetch('GET', 'https://example.com/', ''); }",
                "http_hosts": ["example.com"]
            }),
        )
        .await
        .unwrap();
        promote(d, "fetcher", Some(7)).await.unwrap();
        let tool = load(d, "fetcher").unwrap();
        assert_eq!(tool.granted.http_hosts, vec!["example.com".to_string()]);
        assert!(tool.granted_until.unwrap() > Utc::now() + chrono::Duration::days(6));
        let mut lapsed = tool.clone();
        lapsed.granted_until = Some(Utc::now() - chrono::Duration::days(1));
        save(d, &lapsed).unwrap();
        let err = call(d, "fetcher", &json!({}), None).await.unwrap_err();
        assert!(err.contains("no http capability"), "{err}");
        revoke(d, "fetcher").unwrap();
        assert!(load(d, "fetcher").unwrap().granted.is_empty());
    }

    /// Composition: one written tool calls another it may call, and the data stays in the sandbox.
    #[tokio::test]
    async fn a_tool_calls_another_approved_tool() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        create(
            d,
            "",
            &json!({ "name": "count_words", "description": "Count words.", "script": SCRIPT }),
        )
        .await
        .unwrap();
        promote(d, "count_words", None).await.unwrap();
        create(
            d,
            "",
            &json!({
                "name": "words_report",
                "description": "Report how long a text is.",
                "script": "function main(a) { const n = tool('count_words', { text: a.text }).words; return n > 3 ? 'long' : 'short'; }",
                "tools": ["count_words"]
            }),
        )
        .await
        .unwrap();
        promote(d, "words_report", None).await.unwrap();
        assert_eq!(
            call(
                d,
                "words_report",
                &json!({ "text": "one two three four five" }),
                None
            )
            .await
            .unwrap(),
            "\"long\""
        );
        // Without the capability, the call is refused.
        let mut narrow = load(d, "words_report").unwrap();
        narrow.granted.tools.clear();
        save(d, &narrow).unwrap();
        let err = call(d, "words_report", &json!({ "text": "a" }), None)
            .await
            .unwrap_err();
        assert!(err.contains("no capability to call"), "{err}");
    }

    #[test]
    fn each_tool_has_a_rate_limit() {
        let name = "rate_limited_tool_for_test";
        assert!((0..CALLS_PER_MINUTE).all(|_| within_rate(name)));
        assert!(!within_rate(name));
        assert!(within_rate("another_tool_for_test"));
    }

    #[tokio::test]
    async fn the_app_s_own_names_and_bad_scripts_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let named = |name: &str| json!({ "name": name, "description": "x", "script": SCRIPT });
        assert!(create(dir.path(), "", &named("remember")).await.is_err());
        assert!(create(dir.path(), "", &named("Bad-Name")).await.is_err());
        assert!(create(
            dir.path(),
            "",
            &json!({ "name": "no_main", "description": "x", "script": "1 + 1" })
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn the_test_run_has_no_capabilities_whatever_was_asked() {
        let dir = tempfile::tempdir().unwrap();
        let out = create(
            dir.path(),
            "",
            &json!({
                "name": "fetcher",
                "description": "Fetch a page.",
                "script": "function main() { return fetch('GET', 'https://example.com/', ''); }",
                "http_hosts": ["example.com"]
            }),
        )
        .await
        .unwrap();
        assert!(out.contains("no http capability"), "{out}");
        let tool = load(dir.path(), "fetcher").unwrap();
        assert_eq!(tool.requested.http_hosts, vec!["example.com".to_string()]);
        assert!(tool.granted.is_empty());
    }
}
