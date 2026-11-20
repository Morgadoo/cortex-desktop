//! Running a tool: its rate limit, retiring it, and the call itself within a run.

use super::*;
use crate::chat::error::ChatError;

/// Calls per tool per minute; past it, calls are refused until the minute passes.
pub(super) const CALLS_PER_MINUTE: usize = 30;

/// Per data folder and tool: a trial's copy does not use up the live tool's minute.
pub(super) type Calls = std::sync::Mutex<
    std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>,
>;

pub(super) fn rate_calls() -> &'static Calls {
    static CALLS: std::sync::OnceLock<Calls> = std::sync::OnceLock::new();
    CALLS.get_or_init(Default::default)
}

/// Drops a folder's rate counters, for a folder that is going away (a trial's copy).
pub(in crate::chat) fn forget_rates(config_dir: &Path) {
    let prefix = format!("{}:", config_dir.display());
    rate_calls()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|key, _| !key.starts_with(&prefix));
}

pub(super) fn within_rate(name: &str) -> bool {
    use std::time::{Duration, Instant};
    let mut calls = rate_calls().lock().unwrap_or_else(|p| p.into_inner());
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
pub(crate) fn retire(config_dir: &Path, name: &str) -> Result<(), ChatError> {
    let mut tool = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    tool.status = Status::Deprecated;
    // A file changed outside the app is retired with no access and no approval, and left
    // unsigned.
    if !verifies(config_dir, &tool) {
        tool.granted = Capabilities::default();
        tool.expires.clear();
        tool.approved_hash = None;
        tool.approved_at = None;
    }
    save_taking_away(config_dir, &tool)?;
    crate::chat::traces::set_status(config_dir, "script_tool", name, Status::Deprecated.as_str());
    Ok(())
}

tokio::task_local! {
    /// The run a tool call belongs to, set by the engine around the call: a written tool's runs,
    /// and those of the tools it calls, are recorded in that run's trace.
    static RUN: String;
}

/// Runs `call` as part of `run`'s trace.
pub(in crate::chat) async fn in_run<F: std::future::Future>(run: String, call: F) -> F::Output {
    RUN.scope(run, call).await
}

/// How deep tools may call tools: a script calling a tool that calls a tool, and no further.
pub(super) const MAX_DEPTH: usize = 3;

/// Runs an active tool with its granted capabilities, after checking its script is the one the
/// user approved. `workspace` is the chat's folder for `read`. A script may call the other tools
/// its capabilities name, with the same checks, a few levels deep.
pub async fn call(
    config_dir: &Path,
    name: &str,
    args: &Value,
    workspace: Option<&Path>,
) -> Result<String, ChatError> {
    call_at(
        config_dir.to_path_buf(),
        name.to_string(),
        args.clone(),
        workspace.map(Path::to_path_buf),
        0,
    )
    .await
}

pub(super) fn call_at(
    config_dir: PathBuf,
    name: String,
    args: Value,
    workspace: Option<PathBuf>,
    depth: usize,
) -> futures::future::BoxFuture<'static, Result<String, ChatError>> {
    Box::pin(async move {
        sandbox::cache_in(&config_dir);
        if depth >= MAX_DEPTH {
            return Err(ChatError::Invalid(format!(
                "`{name}` is called too deep; tools call tools {MAX_DEPTH} levels at most."
            )));
        }
        let tool = load(&config_dir, &name)
            .ok_or_else(|| ChatError::Invalid("That tool was removed.".into()))?;
        if tool.status != Status::Active {
            return Err(ChatError::Invalid(
                "That tool is not active; promote it first.".into(),
            ));
        }
        if tool.approved_hash.as_deref() != Some(approval_hash(&tool).as_str()) {
            return Err(ChatError::Invalid("This tool's script, description or parameters changed since it was approved; promote it again.".into()));
        }
        if !verifies(&config_dir, &tool) {
            return Err(ChatError::Invalid("This tool's file was changed outside the app, so its approval no longer holds; promote it again.".into()));
        }
        if !within_rate(&format!("{}:{name}", config_dir.display())) {
            return Err(ChatError::Invalid(format!(
                "`{name}` was called {CALLS_PER_MINUTE} times in the last minute; wait before calling it again."
            )));
        }
        // Each capability granted for a time lapses on its own; the tool can still compute.
        let granted = active_grants(&tool, Utc::now());
        let caller: sandbox::ToolCaller = {
            let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
            std::sync::Arc::new(move |callee: String, input: String| {
                let (config_dir, workspace) = (config_dir.clone(), workspace.clone());
                Box::pin(async move {
                    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
                    // The sandbox takes a nested call's error as text.
                    call_at(config_dir, callee, input, workspace, depth + 1)
                        .await
                        .map_err(String::from)
                })
            })
        };
        let workspace = workspace.map(|root| sandbox::Workspace {
            root,
            // Files the app sealed at rest are opened with its key, and what is written is sealed.
            read: |_, bytes| crate::chat::at_rest::open_bytes(bytes),
            write: |p, bytes| crate::chat::at_rest::seal_for(p, bytes),
        });
        // The vault secrets its granted credentials name, for hosts it may reach: the host
        // attaches them to requests, and they are masked in whatever comes back.
        let mut secrets = std::collections::HashMap::new();
        for credential in &granted.credentials {
            let Some((host, secret)) = credential.split_once('=') else {
                continue;
            };
            if granted
                .http_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(host))
            {
                if let Some(value) = crate::chat::vault::value(&config_dir, secret).await {
                    secrets.insert(secret.to_string(), value);
                }
            }
        }
        let shown: Vec<(String, String)> = secrets.clone().into_iter().collect();
        let started = std::time::Instant::now();
        let (result, logs) = sandbox::run_with_secrets(
            &tool.script,
            &args.to_string(),
            &granted,
            Limits::default(),
            workspace,
            Some(caller),
            secrets,
        )
        .await;
        // Its run, ok or not, in the trace: what the lifecycle and the monitor judge it by. A call
        // from another tool counts too, so a tool used only that way is not retired as unused.
        crate::chat::traces::tool_call(
            &config_dir,
            &RUN.try_with(Clone::clone).unwrap_or_default(),
            &format!("{CONNECTOR}__{name}"),
            started.elapsed().as_millis() as u64,
            result.is_ok(),
        );
        // The tool's own lines are untrusted text: they go to the log file, never to the model.
        for line in logs {
            log::info!("[script tool {name}] {line}");
        }
        // What it returns is scanned for secrets (keys, tokens, passwords) before anything reads
        // it, whatever host it fetched them from.
        // Masked whether it succeeded or failed: an error can quote a response, and with it a
        // credential. A callee's result is masked before it reaches its caller.
        let mask = |text: String| {
            let text = crate::chat::vault::mask(&text, &shown);
            let (masked, found) = cortex_kernel::privacy::mask_secrets(&text);
            if found > 0 {
                log::warn!("[script tool {name}] {found} secret(s) masked in its output");
            }
            masked
        };
        let result = result.map(mask).map_err(|e| mask(e.to_string()));
        // A call from another tool does not pass the chat's own record: it is logged here, so the
        // activity log shows every tool that ran.
        if depth > 0 {
            crate::chat::audit::record(
                &config_dir,
                crate::chat::audit::Entry {
                    at: Utc::now(),
                    kind: "tool".into(),
                    conversation: String::new(),
                    agent: None,
                    tool: format!("{CONNECTOR}__{name} (called by a tool)"),
                    arguments: crate::chat::audit::short(&args.to_string()),
                    outcome: if result.is_ok() { "ok" } else { "error" }.into(),
                    detail: crate::chat::audit::short(match &result {
                        Ok(text) | Err(text) => text,
                    }),
                    millis: started.elapsed().as_millis() as u64,
                },
            );
        }
        result.map_err(ChatError::Invalid)
    })
}
