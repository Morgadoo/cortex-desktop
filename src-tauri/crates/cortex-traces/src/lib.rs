//! Run traces and the artifact registry: what each turn did, what it cost, and how often each
//! tool, skill and memory is used, in a SQLite database at `chat/traces.db`.
//!
//! Only measurements go here: names, ids, times, token counts, prices and whether something
//! failed. Arguments, results and messages do not: they stay in the encrypted conversation files
//! and activity log, since this database is not sealed at rest. That is enough to answer "which
//! tools failed most this week, and what did they cost", for the user and for the model (the
//! `usage_stats` and `failure_summary` built-ins).
//!
//! No tool can write here; only the engine records. Losing the file loses statistics, never data.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    conversation TEXT NOT NULL,
    agent TEXT,
    model TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    outcome TEXT,
    steps INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS model_calls (
    run TEXT NOT NULL,
    step INTEGER NOT NULL,
    model TEXT NOT NULL,
    at INTEGER NOT NULL,
    millis INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost REAL NOT NULL,
    error TEXT
);
CREATE TABLE IF NOT EXISTS tool_calls (
    run TEXT NOT NULL,
    tool TEXT NOT NULL,
    at INTEGER NOT NULL,
    millis INTEGER NOT NULL,
    ok INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts (
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    version INTEGER NOT NULL DEFAULT 1,
    hash TEXT,
    provenance TEXT,
    critical INTEGER NOT NULL DEFAULT 0,
    loaded INTEGER NOT NULL DEFAULT 0,
    used INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_used INTEGER,
    PRIMARY KEY (kind, id)
);
CREATE TABLE IF NOT EXISTS feedback (
    conversation TEXT NOT NULL,
    message INTEGER NOT NULL,
    rating INTEGER NOT NULL,
    model TEXT,
    agent TEXT,
    at INTEGER NOT NULL,
    PRIMARY KEY (conversation, message)
);
CREATE TABLE IF NOT EXISTS run_artifacts (
    run TEXT NOT NULL,
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    used INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run, kind, id)
);
CREATE TABLE IF NOT EXISTS signals (
    conversation TEXT NOT NULL,
    message INTEGER,
    kind TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_by_time ON runs (started_at);
CREATE INDEX IF NOT EXISTS tools_by_time ON tool_calls (at);
CREATE INDEX IF NOT EXISTS models_by_time ON model_calls (at);
";

fn chat_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("chat")
}

/// What one model call used, as the provider reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spent {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// US dollars, when the model is priced.
    pub cost: f64,
}

/// A data folder's database and its connection, kept for the life of the process.
type Open = (PathBuf, &'static Mutex<Connection>);

/// One connection per data folder, opened on first use.
fn connection(config_dir: &Path) -> Option<&'static Mutex<Connection>> {
    static OPEN: OnceLock<Mutex<Vec<Open>>> = OnceLock::new();
    let path = chat_dir(config_dir).join("traces.db");
    let mut open = OPEN
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some((_, conn)) = open.iter().find(|(p, _)| *p == path) {
        return Some(conn);
    }
    std::fs::create_dir_all(path.parent()?).ok()?;
    let conn = Connection::open(&path)
        .map_err(|e| log::warn!("could not open the trace database: {e}"))
        .ok()?;
    conn.execute_batch(&format!("PRAGMA journal_mode=WAL; {SCHEMA}"))
        .map_err(|e| log::warn!("could not prepare the trace database: {e}"))
        .ok()?;
    // Added after the first release: why the user rated an answer as they did. Fails harmlessly
    // when the column is already there.
    let _ = conn.execute("ALTER TABLE feedback ADD COLUMN reason TEXT", []);
    let conn: &'static Mutex<Connection> = Box::leak(Box::new(Mutex::new(conn)));
    open.push((path, conn));
    Some(conn)
}

/// Runs `f` on the database; failures are logged and swallowed, since statistics must never stop
/// a turn.
fn with<T>(config_dir: &Path, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let conn = connection(config_dir)?;
    let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
    f(&conn).map_err(|e| log::warn!("trace database: {e}")).ok()
}

fn now() -> i64 {
    Utc::now().timestamp_millis()
}

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
    attribute(config_dir, run, "tool", tool, true);
}

/// The artifact registry's row for something the assistant can use or change.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub kind: String,
    pub id: String,
    /// `candidate`, `active`, `deprecated` or `archived`.
    pub status: String,
    pub version: i64,
    pub hash: Option<String>,
    /// Who made it: `user`, or the conversation the assistant made it in.
    pub provenance: Option<String>,
    /// Never pruned for disuse, however rarely it is used.
    pub critical: bool,
    pub loaded: i64,
    pub used: i64,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
}

fn ensure(c: &Connection, kind: &str, id: &str) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT OR IGNORE INTO artifacts (kind, id, created_at) VALUES (?1, ?2, ?3)",
        params![kind, id, now()],
    )
}

/// Records that an artifact was put in front of the model (a skill listed, a page included).
pub fn loaded(config_dir: &Path, kind: &str, ids: &[String]) {
    with(config_dir, |c| {
        for id in ids {
            ensure(c, kind, id)?;
            c.execute(
                "UPDATE artifacts SET loaded = loaded + 1 WHERE kind = ?1 AND id = ?2",
                params![kind, id],
            )?;
        }
        Ok(())
    });
}

/// Records that an artifact was used (a tool called, a skill opened).
pub fn used(config_dir: &Path, kind: &str, id: &str) {
    used_at(config_dir, kind, id, now());
}

fn used_at(config_dir: &Path, kind: &str, id: &str, at: i64) {
    with(config_dir, |c| {
        ensure(c, kind, id)?;
        c.execute(
            "UPDATE artifacts SET used = used + 1, last_used = MAX(COALESCE(last_used, 0), ?3) WHERE kind = ?1 AND id = ?2",
            params![kind, id, at],
        )
    });
}

/// The user's rating of an answer: 1 good, -1 bad, 0 withdrawn. Kept per answer, so changing a
/// rating replaces it.
pub fn feedback(
    config_dir: &Path,
    conversation: &str,
    message: usize,
    rating: i64,
    model: Option<&str>,
    agent: Option<&str>,
) {
    with(config_dir, |c| {
        if rating == 0 {
            c.execute(
                "DELETE FROM feedback WHERE conversation = ?1 AND message = ?2",
                params![conversation, message as i64],
            )
        } else {
            c.execute(
                "INSERT OR REPLACE INTO feedback (conversation, message, rating, model, agent, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![conversation, message as i64, rating, model, agent, now()],
            )
        }
    });
}

/// Why the user rated an answer as they did: the strongest signal, an expert's judgement with its
/// reason. Kept beside the rating; an answer without a rating gets none.
pub fn feedback_reason(config_dir: &Path, conversation: &str, message: usize, reason: &str) {
    with(config_dir, |c| {
        c.execute(
            "UPDATE feedback SET reason = ?3 WHERE conversation = ?1 AND message = ?2",
            params![
                conversation,
                message as i64,
                Some(reason.trim()).filter(|r| !r.is_empty())
            ],
        )
    });
}

/// The reasons given with ratings since `since`: (conversation, message, rating, reason).
pub fn rating_reasons(config_dir: &Path, since: DateTime<Utc>) -> Vec<(String, i64, i64, String)> {
    with(config_dir, |c| {
        let mut stmt = c.prepare(
            "SELECT conversation, message, rating, reason FROM feedback WHERE at >= ?1 AND reason IS NOT NULL ORDER BY at",
        )?;
        let rows = stmt
            .query_map(params![since.timestamp_millis()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>();
        rows
    })
    .unwrap_or_default()
}

/// Attribution: records that an artifact was put in front of the model in `run` (`used` false)
/// or used in it (`used` true). A use also counts as having been there.
pub fn attribute(config_dir: &Path, run: &str, kind: &str, id: &str, used: bool) {
    with(config_dir, |c| {
        c.execute(
            "INSERT INTO run_artifacts (run, kind, id, used) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (run, kind, id) DO UPDATE SET used = MAX(used, excluded.used)",
            params![run, kind, id, used],
        )
    });
}

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

/// What people do with an answer, which says something about it even unrated.
pub const SIGNALS: &[&str] = &[
    "regenerated",
    "edited",
    "resumed",
    "copied",
    "exported",
    // A link or source in the answer opened: the answer pointed somewhere worth going.
    "opened_source",
    // A tool that worked but returned nothing: a technical outcome worth counting.
    "empty_result",
];

/// Whether a tool's successful output is empty: nothing, or an empty list, object or string.
pub fn empty_result(text: &str) -> bool {
    matches!(text.trim(), "" | "[]" | "{}" | "null" | "\"\"")
}

/// Records an implicit signal: an answer regenerated, a prompt edited and sent again, a stopped
/// turn resumed, an answer copied or a chat exported. Kinds outside [`SIGNALS`] are ignored.
pub fn signal(config_dir: &Path, conversation: &str, message: Option<usize>, kind: &str) {
    if !SIGNALS.contains(&kind) {
        return;
    }
    with(config_dir, |c| {
        c.execute(
            "INSERT INTO signals (conversation, message, kind, at) VALUES (?1, ?2, ?3, ?4)",
            params![conversation, message.map(|m| m as i64), kind, now()],
        )
    });
}

/// How often each signal happened since `since`.
pub fn signal_counts(config_dir: &Path, since: DateTime<Utc>) -> Vec<(String, i64)> {
    with(config_dir, |c| {
        let mut stmt = c.prepare(
            "SELECT kind, COUNT(*) FROM signals WHERE at >= ?1 GROUP BY kind ORDER BY kind",
        )?;
        let rows = stmt.query_map(params![since.timestamp_millis()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        rows.collect()
    })
    .unwrap_or_default()
}

/// Good and bad ratings per model since `since`.
pub fn ratings(config_dir: &Path, since: DateTime<Utc>) -> Vec<(String, i64, i64)> {
    with(config_dir, |c| {
        let mut stmt = c.prepare(
            "SELECT COALESCE(model, ''), SUM(rating > 0), SUM(rating < 0) FROM feedback WHERE at >= ?1 GROUP BY model ORDER BY model",
        )?;
        let rows = stmt.query_map(params![since.timestamp_millis()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect()
    })
    .unwrap_or_default()
}

/// A short content hash, for telling versions apart.
pub fn hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))[..16].to_string()
}

/// Registers a new version of an artifact, with its content hash and who made it.
pub fn register(
    config_dir: &Path,
    kind: &str,
    id: &str,
    hash: Option<&str>,
    provenance: Option<&str>,
    status: &str,
) {
    with(config_dir, |c| {
        ensure(c, kind, id)?;
        c.execute(
            "UPDATE artifacts SET version = CASE WHEN hash IS NULL OR hash = ?3 THEN version ELSE version + 1 END, hash = ?3, provenance = COALESCE(?4, provenance), status = ?5 WHERE kind = ?1 AND id = ?2",
            params![kind, id, hash, provenance, status],
        )
    });
}

pub fn set_status(config_dir: &Path, kind: &str, id: &str, status: &str) {
    with(config_dir, |c| {
        ensure(c, kind, id)?;
        c.execute(
            "UPDATE artifacts SET status = ?3 WHERE kind = ?1 AND id = ?2",
            params![kind, id, status],
        )
    });
}

pub fn set_critical(config_dir: &Path, kind: &str, id: &str, critical: bool) {
    with(config_dir, |c| {
        ensure(c, kind, id)?;
        c.execute(
            "UPDATE artifacts SET critical = ?3 WHERE kind = ?1 AND id = ?2",
            params![kind, id, critical],
        )
    });
}

fn time(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

pub fn artifact(config_dir: &Path, kind: &str, id: &str) -> Option<Artifact> {
    with(config_dir, |c| {
        c.query_row(
            "SELECT kind, id, status, version, hash, provenance, critical, loaded, used, created_at, last_used FROM artifacts WHERE kind = ?1 AND id = ?2",
            params![kind, id],
            row_artifact,
        )
        .optional()
    })
    .flatten()
}

fn row_artifact(r: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    Ok(Artifact {
        kind: r.get(0)?,
        id: r.get(1)?,
        status: r.get(2)?,
        version: r.get(3)?,
        hash: r.get(4)?,
        provenance: r.get(5)?,
        critical: r.get(6)?,
        loaded: r.get(7)?,
        used: r.get(8)?,
        created_at: time(r.get(9)?),
        last_used: r.get::<_, Option<i64>>(10)?.map(time),
    })
}

pub fn artifacts(config_dir: &Path, kind: Option<&str>) -> Vec<Artifact> {
    with(config_dir, |c| {
        let mut stmt = c.prepare(
            "SELECT kind, id, status, version, hash, provenance, critical, loaded, used, created_at, last_used FROM artifacts WHERE ?1 IS NULL OR kind = ?1 ORDER BY kind, id",
        )?;
        let rows = stmt.query_map(params![kind], row_artifact)?;
        rows.collect()
    })
    .unwrap_or_default()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_traced_and_summed_without_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        start_run(d, "r1", "chat-1", Some("agent-a"), "mock/m");
        let spent = Spent {
            input_tokens: 100,
            output_tokens: 20,
            cost: 0.0,
        };
        model_call(d, "r1", 0, "mock/m", 900, Some(spent), None);
        tool_call(d, "r1", "web_search", 300, true);
        tool_call(d, "r1", "web_search", 500, false);
        tool_call(d, "r1", "read_chat", 10, true);
        model_call(d, "r1", 1, "mock/m", 400, None, Some("provider"));
        end_run(d, "r1", "error");

        let u = usage(d, Utc::now() - chrono::Duration::days(1));
        assert_eq!((u.runs, u.failed_runs), (1, 1));
        assert_eq!(u.tools[0].tool, "web_search");
        assert_eq!((u.tools[0].calls, u.tools[0].failures), (2, 1));
        assert_eq!(u.tools[0].avg_ms, 400);
        assert_eq!(u.models[0].input_tokens, 100);
        assert_eq!(u.models[0].failures, 1);
        assert!(failure_report(d, 7).contains("web_search"));
        assert!(!failure_report(d, 7).contains("read_chat"));
        assert_eq!(
            agent_cost(d, "agent-a", Utc::now() - chrono::Duration::days(1)),
            Some(0.0)
        );
        assert_eq!(
            agent_cost(d, "agent-b", Utc::now() - chrono::Duration::days(1)),
            None
        );
        // A tool call counts as a use of the tool.
        assert_eq!(artifact(d, "tool", "web_search").unwrap().used, 2);
    }

    #[test]
    fn ratings_are_kept_per_answer_and_counted_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        feedback(d, "c", 1, 1, Some("m/a"), None);
        feedback(d, "c", 1, -1, Some("m/a"), None);
        feedback(d, "c", 3, 1, Some("m/a"), None);
        feedback(d, "c", 5, 1, Some("m/b"), None);
        feedback(d, "c", 5, 0, Some("m/b"), None);
        let since = Utc::now() - chrono::Duration::days(1);
        assert_eq!(ratings(d, since), vec![("m/a".to_string(), 1, 1)]);
        // Expert validation: the reason goes with the rating.
        feedback_reason(d, "c", 1, " It cited the wrong year. ");
        feedback_reason(d, "c", 9, "no rating, so not kept");
        assert_eq!(
            rating_reasons(d, since),
            vec![(
                "c".to_string(),
                1,
                -1,
                "It cited the wrong year.".to_string()
            )]
        );
    }

    #[test]
    fn runs_can_be_queried_and_opened_with_what_they_used() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        start_run(d, "run-a", "chat-1", None, "m");
        tool_call(d, "run-a", "web_search", 10, false);
        attribute(d, "run-a", "skill", "summarize", false);
        attribute(d, "run-a", "skill", "summarize", true);
        attribute(d, "run-a", "skill", "translate", false);
        end_run(d, "run-a", "done");
        start_run(d, "run-b", "chat-2", None, "m");
        tool_call(d, "run-b", "read_chat", 10, true);
        end_run(d, "run-b", "error");
        let failing = runs(
            d,
            &RunFilter {
                failed_tools: true,
                limit: 10,
                ..RunFilter::default()
            },
        );
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].id, "run-a");
        let errors = runs(
            d,
            &RunFilter {
                outcome: Some("error".into()),
                limit: 10,
                ..RunFilter::default()
            },
        );
        assert_eq!(errors[0].id, "run-b");
        let detail = run_detail(d, "run-a").unwrap();
        assert_eq!(
            detail.tool_calls,
            vec![("web_search".to_string(), 10, false)]
        );
        assert_eq!(
            detail.artifacts,
            vec![
                ("skill".to_string(), "summarize".to_string(), true),
                ("skill".to_string(), "translate".to_string(), false),
                // A tool called in the run is attributed to it as used.
                ("tool".to_string(), "web_search".to_string(), true)
            ]
        );
    }

    #[test]
    fn implicit_signals_are_counted_and_unknown_kinds_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        signal(d, "c", Some(3), "regenerated");
        signal(d, "c", Some(3), "regenerated");
        signal(d, "c", None, "exported");
        signal(d, "c", None, "deleted_everything");
        let counts = signal_counts(d, Utc::now() - chrono::Duration::days(1));
        assert_eq!(
            counts,
            vec![("exported".into(), 1), ("regenerated".into(), 2)]
        );
        assert!(usage_report(d, 1).contains("regenerated"));
        // A tool that worked and returned nothing is a technical signal.
        for empty in ["", " [] ", "{}", "null", "\"\""] {
            assert!(empty_result(empty), "{empty:?}");
        }
        assert!(!empty_result("[1]") && !empty_result("0"));
    }

    #[test]
    fn artifacts_keep_versions_status_and_use() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        register(
            d,
            "skill",
            "summarize",
            Some("h1"),
            Some("chat-9"),
            "candidate",
        );
        register(d, "skill", "summarize", Some("h1"), None, "candidate");
        assert_eq!(artifact(d, "skill", "summarize").unwrap().version, 1);
        register(d, "skill", "summarize", Some("h2"), None, "active");
        let a = artifact(d, "skill", "summarize").unwrap();
        assert_eq!((a.version, a.status.as_str()), (2, "active"));
        assert_eq!(a.provenance.as_deref(), Some("chat-9"));
        loaded(d, "skill", &["summarize".into()]);
        used(d, "skill", "summarize");
        let a = artifact(d, "skill", "summarize").unwrap();
        assert_eq!((a.loaded, a.used), (1, 1));
        assert!(a.last_used.is_some());
        set_critical(d, "skill", "summarize", true);
        set_status(d, "skill", "summarize", "deprecated");
        let a = artifact(d, "skill", "summarize").unwrap();
        assert!(a.critical);
        assert_eq!(a.status, "deprecated");
        assert_eq!(artifacts(d, Some("skill")).len(), 1);
    }
}
