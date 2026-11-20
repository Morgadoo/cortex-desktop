//! Reading runs back: lists with filters, and one run in detail.

use super::*;

/// A run, as `query_traces` lists it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    pub id: String,
    pub conversation: String,
    pub agent: Option<String>,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub outcome: Option<String>,
    pub steps: i64,
    pub cost: f64,
    pub tool_calls: i64,
    pub failed_tool_calls: i64,
}

/// Which runs to list.
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    pub since: Option<DateTime<Utc>>,
    /// `done`, `cancelled` or `error`.
    pub outcome: Option<String>,
    /// Only runs that called this tool.
    pub tool: Option<String>,
    /// Only runs with a failed tool call.
    pub failed_tools: bool,
    pub limit: usize,
}

/// Runs matching `filter`, newest first.
pub fn runs(config_dir: &Path, filter: &RunFilter) -> Vec<RunRow> {
    with(config_dir, |c| {
        let mut stmt = c.prepare(
            "SELECT r.id, r.conversation, r.agent, r.model, r.started_at, r.outcome, r.steps, r.cost,
                    (SELECT COUNT(*) FROM tool_calls t WHERE t.run = r.id),
                    (SELECT COUNT(*) FROM tool_calls t WHERE t.run = r.id AND t.ok = 0)
             FROM runs r
             WHERE r.started_at >= ?1
               AND (?2 IS NULL OR r.outcome = ?2)
               AND (?3 IS NULL OR EXISTS (SELECT 1 FROM tool_calls t WHERE t.run = r.id AND t.tool = ?3))
               AND (?4 = 0 OR EXISTS (SELECT 1 FROM tool_calls t WHERE t.run = r.id AND t.ok = 0))
             ORDER BY r.started_at DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![
                filter.since.map_or(0, |s| s.timestamp_millis()),
                filter.outcome,
                filter.tool,
                filter.failed_tools,
                filter.limit.clamp(1, 200) as i64
            ],
            |r| {
                Ok(RunRow {
                    id: r.get(0)?,
                    conversation: r.get(1)?,
                    agent: r.get(2)?,
                    model: r.get(3)?,
                    started_at: time(r.get(4)?),
                    outcome: r.get(5)?,
                    steps: r.get(6)?,
                    cost: r.get(7)?,
                    tool_calls: r.get(8)?,
                    failed_tool_calls: r.get(9)?,
                })
            },
        )?;
        rows.collect()
    })
    .unwrap_or_default()
}

/// One run in full: its model calls, tool calls, and what it had in front of it and used.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub run: RunRow,
    /// (step, model, millis, input tokens, output tokens, error)
    pub model_calls: Vec<(i64, String, i64, i64, i64, Option<String>)>,
    /// (tool, millis, ok)
    pub tool_calls: Vec<(String, i64, bool)>,
    /// (kind, id, used)
    pub artifacts: Vec<(String, String, bool)>,
}

pub fn run_detail(config_dir: &Path, run: &str) -> Option<RunDetail> {
    let row = runs(
        config_dir,
        &RunFilter {
            limit: 200,
            ..RunFilter::default()
        },
    )
    .into_iter()
    .find(|r| r.id == run || r.id.starts_with(run))?;
    let id = row.id.clone();
    with(config_dir, |c| {
        let mut stmt = c.prepare("SELECT step, model, millis, input_tokens, output_tokens, error FROM model_calls WHERE run = ?1 ORDER BY at")?;
        let model_calls = stmt
            .query_map(params![id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt =
            c.prepare("SELECT tool, millis, ok FROM tool_calls WHERE run = ?1 ORDER BY at")?;
        let tool_calls = stmt
            .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt =
            c.prepare("SELECT kind, id, used FROM run_artifacts WHERE run = ?1 ORDER BY kind, id")?;
        let artifacts = stmt
            .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(RunDetail {
            run: row,
            model_calls,
            tool_calls,
            artifacts,
        })
    })
}
