//! Recording a run as it happens: its start and end, each model call and each tool call.

use super::*;

pub fn start_run(
    config_dir: &Path,
    run: &str,
    conversation: &str,
    agent: Option<&str>,
    model: &str,
) {
    with(config_dir, |c| {
        c.execute(
            "INSERT OR REPLACE INTO runs (id, conversation, agent, model, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run, conversation, agent, model, now()],
        )
    });
}

/// `outcome` is `done`, `cancelled` or `error`.
pub fn end_run(config_dir: &Path, run: &str, outcome: &str) {
    with(config_dir, |c| {
        c.execute(
            "UPDATE runs SET ended_at = ?2, outcome = ?3 WHERE id = ?1",
            params![run, now(), outcome],
        )
    });
}

/// One model call: how long it took, what it used, and the kind of error if it failed.
#[allow(clippy::too_many_arguments)]
pub fn model_call(
    config_dir: &Path,
    run: &str,
    step: usize,
    model: &str,
    millis: u64,
    spent: Option<Spent>,
    error: Option<&str>,
) {
    let spent = spent.unwrap_or_default();
    let (input, output, cost) = (
        spent.input_tokens as i64,
        spent.output_tokens as i64,
        spent.cost,
    );
    with(config_dir, |c| {
        c.execute(
            "INSERT INTO model_calls (run, step, model, at, millis, input_tokens, output_tokens, cost, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![run, step as i64, model, now(), millis as i64, input, output, cost, error],
        )?;
        c.execute(
            "UPDATE runs SET steps = MAX(steps, ?2 + 1), input_tokens = input_tokens + ?3, output_tokens = output_tokens + ?4, cost = cost + ?5 WHERE id = ?1",
            params![run, step as i64, input, output, cost],
        )
    });
}

pub fn tool_call(config_dir: &Path, run: &str, tool: &str, millis: u64, ok: bool) {
    tool_call_at(config_dir, run, tool, Utc::now(), millis, ok);
}

/// [`tool_call`], at a given time.
pub fn tool_call_at(
    config_dir: &Path,
    run: &str,
    tool: &str,
    at: DateTime<Utc>,
    millis: u64,
    ok: bool,
) {
    let at = at.timestamp_millis();
    with(config_dir, |c| {
        c.execute(
            "INSERT INTO tool_calls (run, tool, at, millis, ok) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run, tool, at, millis as i64, ok],
        )
    });
    used_at(config_dir, "tool", tool, at);
    // A tool the assistant wrote is also its own artifact, which the lifecycle judges by use.
    if let Some(name) = tool.strip_prefix("script__") {
        used_at(config_dir, "script_tool", name, at);
    }
    // A run of no turn (a tool called by another tool) counts as a use, attributed to no run.
    if !run.is_empty() {
        attribute(config_dir, run, "tool", tool, true);
    }
}
