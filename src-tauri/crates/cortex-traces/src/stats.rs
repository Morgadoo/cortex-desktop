//! Statistics from the traces: per tool, per model, usage and cost, and failures.

use super::*;

/// Per tool, over a period: calls, failures and average time.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolStat {
    pub tool: String,
    pub calls: i64,
    pub failures: i64,
    pub avg_ms: i64,
}

/// Per model, over a period: calls, failures, tokens and cost.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelStat {
    pub model: String,
    pub calls: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Usage {
    pub since: DateTime<Utc>,
    pub runs: i64,
    pub failed_runs: i64,
    pub cost: f64,
    pub tools: Vec<ToolStat>,
    pub models: Vec<ModelStat>,
}

/// One tool's calls and failures between `from` and `to`.
pub fn tool_stat(
    config_dir: &Path,
    tool: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> ToolStat {
    let empty = ToolStat {
        tool: tool.to_string(),
        calls: 0,
        failures: 0,
        avg_ms: 0,
    };
    with(config_dir, |c| {
        c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(ok = 0), 0), CAST(COALESCE(AVG(millis), 0) AS INTEGER) FROM tool_calls WHERE tool = ?1 AND at >= ?2 AND at < ?3",
            params![tool, from.timestamp_millis(), to.timestamp_millis()],
            |r| {
                Ok(ToolStat {
                    tool: tool.to_string(),
                    calls: r.get(0)?,
                    failures: r.get(1)?,
                    avg_ms: r.get(2)?,
                })
            },
        )
    })
    .unwrap_or(empty)
}

/// Usage since `since`: runs, cost, and per tool and model, most used first.
pub fn usage(config_dir: &Path, since: DateTime<Utc>) -> Usage {
    let from = since.timestamp_millis();
    let empty = Usage {
        since,
        runs: 0,
        failed_runs: 0,
        cost: 0.0,
        tools: Vec::new(),
        models: Vec::new(),
    };
    with(config_dir, |c| {
        let (runs, failed_runs, cost): (i64, i64, f64) = c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(outcome = 'error'), 0), COALESCE(SUM(cost), 0) FROM runs WHERE started_at >= ?1",
            params![from],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut stmt = c.prepare(
            "SELECT tool, COUNT(*), SUM(ok = 0), CAST(AVG(millis) AS INTEGER) FROM tool_calls WHERE at >= ?1 GROUP BY tool ORDER BY COUNT(*) DESC, tool",
        )?;
        let tools = stmt
            .query_map(params![from], |r| {
                Ok(ToolStat {
                    tool: r.get(0)?,
                    calls: r.get(1)?,
                    failures: r.get(2)?,
                    avg_ms: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = c.prepare(
            "SELECT model, COUNT(*), SUM(error IS NOT NULL), SUM(input_tokens), SUM(output_tokens), SUM(cost) FROM model_calls WHERE at >= ?1 GROUP BY model ORDER BY COUNT(*) DESC, model",
        )?;
        let models = stmt
            .query_map(params![from], |r| {
                Ok(ModelStat {
                    model: r.get(0)?,
                    calls: r.get(1)?,
                    failures: r.get(2)?,
                    input_tokens: r.get(3)?,
                    output_tokens: r.get(4)?,
                    cost: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Usage {
            since,
            runs,
            failed_runs,
            cost,
            tools,
            models,
        })
    })
    .unwrap_or(empty)
}

/// What an agent's runs cost since `since`, or `None` when nothing was traced.
pub fn agent_cost(config_dir: &Path, agent: &str, since: DateTime<Utc>) -> Option<f64> {
    with(config_dir, |c| {
        c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(cost), 0) FROM runs WHERE agent = ?1 AND started_at >= ?2",
            params![agent, since.timestamp_millis()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
        )
    })
    .and_then(|(runs, cost)| (runs > 0).then_some(cost))
}

/// The usage report the model reads, as compact JSON.
pub fn usage_report(config_dir: &Path, days: i64) -> String {
    let since = Utc::now() - chrono::Duration::days(days.clamp(1, 365));
    let ratings: Vec<serde_json::Value> = ratings(config_dir, since)
        .into_iter()
        .map(|(model, good, bad)| serde_json::json!({ "model": model, "good": good, "bad": bad }))
        .collect();
    let signals: serde_json::Map<String, serde_json::Value> = signal_counts(config_dir, since)
        .into_iter()
        .map(|(kind, n)| (kind, n.into()))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "usage": usage(config_dir, since),
        "ratings": ratings,
        "signals": signals,
    }))
    .unwrap_or_default()
}

/// What failed over the period, most often first: tools with failures and models with errors.
pub fn failure_report(config_dir: &Path, days: i64) -> String {
    let since = Utc::now() - chrono::Duration::days(days.clamp(1, 365));
    let u = usage(config_dir, since);
    let tools: Vec<&ToolStat> = {
        let mut t: Vec<&ToolStat> = u.tools.iter().filter(|t| t.failures > 0).collect();
        t.sort_by(|a, b| b.failures.cmp(&a.failures).then(a.tool.cmp(&b.tool)));
        t
    };
    let models: Vec<&ModelStat> = u.models.iter().filter(|m| m.failures > 0).collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "since": since,
        "runs": u.runs,
        "failedRuns": u.failed_runs,
        "failingTools": tools,
        "failingModels": models,
    }))
    .unwrap_or_default()
}
