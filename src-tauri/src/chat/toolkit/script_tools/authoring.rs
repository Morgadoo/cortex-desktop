//! Writing a tool: the draft, its checks, near-duplicates and taken names, and creating it as a candidate.

use super::*;
use crate::chat::error::ChatError;

/// What `create_tool` asks for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct Draft {
    pub(super) name: String,
    pub(super) description: String,
    #[serde(default)]
    pub(super) parameters: Option<Value>,
    pub(super) script: String,
    #[serde(default)]
    pub(super) test_args: Option<Value>,
    #[serde(default)]
    pub(super) http_hosts: Vec<String>,
    #[serde(default)]
    pub(super) read_prefixes: Vec<String>,
    #[serde(default)]
    pub(super) write_prefixes: Vec<String>,
    /// Vault secrets for its requests, as `host=SECRET`.
    #[serde(default)]
    pub(super) credentials: Vec<String>,
    /// Other approved tools it calls with `tool(name, args)`.
    #[serde(default)]
    pub(super) tools: Vec<String>,
    /// Save it even though a similar tool exists.
    #[serde(default)]
    pub(super) new_anyway: bool,
}

/// An existing tool that does what `description` says, if one does: the kernel's rule is to
/// search before creating and to update a tool rather than add a near-duplicate.
pub(super) fn near_duplicate<'a>(
    tools: &'a [ScriptTool],
    name: &str,
    description: &str,
) -> Option<&'a ScriptTool> {
    use std::collections::HashSet;
    let words = |t: &str| -> HashSet<String> {
        crate::chat::retrieval::tokens(t)
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
pub(super) async fn failing_checks(script: &str, checks: &[Check]) -> Vec<String> {
    let mut failed = Vec::new();
    for check in checks {
        // A fixed clock and seeded randomness, as when the check was recorded: a tool that reads
        // the date or draws random numbers still gives the same result tomorrow.
        let (result, _) = sandbox::run(
            script,
            &check.args.to_string(),
            &Capabilities::default(),
            Limits {
                deterministic: true,
                ..Limits::default()
            },
            None,
        )
        .await;
        match result {
            Ok(out) if out == check.expected => {}
            Ok(out) => failed.push(format!(
                "{} gave {} instead of {}",
                check.args,
                crate::support::text::clip(&out, 200),
                crate::support::text::clip(&check.expected, 200)
            )),
            Err(e) => failed.push(format!("{} failed: {e}", check.args)),
        }
    }
    failed
}

/// Names a written tool may not take: the app's own tools.
pub(super) fn taken(name: &str) -> bool {
    crate::chat::tool_names::NAMES.contains(&name)
        || [
            crate::chat::tool_names::DELEGATE,
            crate::chat::tool_names::CANVAS,
            crate::chat::tool_names::NOTIFY,
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
pub const RUN_SCRIPT: &str = "run_script";

/// The kernel's `run(code, limits)`: a one-off JavaScript program in the sandbox, with no access
/// to anything, within the limits asked for (capped at the defaults). Nothing is saved.
pub async fn run_once(config_dir: &Path, args: &Value) -> Result<String, ChatError> {
    sandbox::cache_in(config_dir);
    let code = args
        .get("code")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| ChatError::Invalid("Give the program as `code`.".into()))?;
    let input = args.get("args").cloned().unwrap_or_else(|| json!({}));
    let defaults = Limits::default();
    let limits = Limits {
        timeout: args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .map_or(defaults.timeout, |s| {
                std::time::Duration::from_secs(s.clamp(1, defaults.timeout.as_secs()))
            }),
        memory_bytes: args
            .get("memory_mb")
            .and_then(Value::as_u64)
            .map_or(defaults.memory_bytes, |mb| {
                ((mb.clamp(16, 128) as usize) << 20).min(defaults.memory_bytes)
            }),
        ..defaults
    };
    let (result, _logs) = sandbox::run(
        code,
        &input.to_string(),
        &Capabilities::default(),
        limits,
        None,
    )
    .await;
    result
        .map(|out| cortex_kernel::privacy::mask_secrets(&out).0)
        .map_err(ChatError::internal)
}

/// `create_tool`: saves (or replaces) a candidate and runs it once on its test arguments, with no
/// capabilities. A promoted tool that is rewritten goes back to being a candidate.
pub async fn create(
    config_dir: &Path,
    conversation: &str,
    args: &Value,
) -> Result<String, ChatError> {
    sandbox::cache_in(config_dir);
    // Signed from its first save: a tool written before the key was loaded would otherwise look,
    // once it is, like a file changed outside the app.
    #[cfg(not(test))]
    ensure_signing_key(config_dir).await;
    let draft: Draft = serde_json::from_value(args.clone())
        .map_err(|e| ChatError::Invalid(format!("The tool is not described right: {e}")))?;
    let name = draft.name.trim().to_string();
    if taken(&name) {
        return Err(ChatError::Invalid(format!(
            "`{name}` is one of the app's own tools; choose another name."
        )));
    }
    if draft.description.trim().is_empty() {
        return Err(ChatError::Invalid(
            "Describe what the tool does and when to use it.".into(),
        ));
    }
    if !draft.script.contains("function main") {
        return Err(ChatError::Invalid(
            "The script must define `function main(args)` and return the result.".into(),
        ));
    }
    let parameters = draft
        .parameters
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    if !parameters.is_object() {
        return Err(ChatError::Invalid(
            "`parameters` must be a JSON schema object.".into(),
        ));
    }
    let existing = list(config_dir);
    if !draft.new_anyway && !existing.iter().any(|t| t.name == name) {
        if let Some(similar) = near_duplicate(&existing, &name, &draft.description) {
            return Err(ChatError::Invalid(format!(
                "`{}` already looks like this tool ({}). Update it by calling create_tool with name \
                 `{}`, or, if this is really a different tool, call again with new_anyway: true.",
                similar.name, similar.description, similar.name
            )));
        }
    }
    let previous = existing.into_iter().find(|t| t.name == name);
    // A rewrite keeps the checks and set-aside list the app signed: for a file changed outside
    // the app, those of the last state the app signed.
    let signed_previous = previous.as_ref().and_then(|t| {
        if verifies(config_dir, t) {
            Some(t.clone())
        } else {
            last_signed(config_dir, &name)
        }
    });
    let checks = signed_previous
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
            write_prefixes: draft.write_prefixes,
            tools: draft.tools,
            credentials: draft.credentials,
        },
        granted: Capabilities::default(),
        status: Status::Candidate,
        approved_hash: None,
        created_at: Utc::now(),
        provenance: (!conversation.is_empty()).then(|| conversation.to_string()),
        checks,
        pending_check: None,
        granted_until: None,
        signature: None,
        expires: Default::default(),
        monitor_reverted_at: None,
        approved_at: None,
        // A rewrite keeps what the user set aside; only approving a set-aside version clears it.
        // (From what the app signed: a file changed outside the app plants nothing.)
        set_aside: signed_previous
            .as_ref()
            .map(|t| t.set_aside.clone())
            .unwrap_or_default(),
        restored_at: None,
        seq: 0,
    };
    let mut tool = tool;
    save_resigned(config_dir, &tool)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        &name,
        Some(&hash(&tool.script)),
        tool.provenance.as_deref(),
        Status::Candidate.as_str(),
    );
    // The test run: no capabilities, whatever the tool asked for, and deterministic, since what
    // it gives becomes the tool's regression check.
    let test_args = draft.test_args.unwrap_or_else(|| json!({})).to_string();
    let (result, logs) = sandbox::run(
        &tool.script,
        &test_args,
        &Capabilities::default(),
        Limits {
            deterministic: true,
            ..Limits::default()
        },
        None,
    )
    .await;
    let test = match result {
        Ok(output) => {
            tool.pending_check = Some(Check {
                args: serde_json::from_str(&test_args).unwrap_or(Value::Null),
                expected: output.clone(),
            });
            save_resigned(config_dir, &tool)?;
            json!({ "ok": true, "output": crate::support::text::clip(&output, 4_000) })
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
