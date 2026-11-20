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

fn opened() -> std::sync::MutexGuard<'static, Vec<Open>> {
    static OPEN: OnceLock<Mutex<Vec<Open>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Closes a data folder's database, for a folder that is going away (a trial's copy): its file
/// is released, and a later use opens it afresh.
pub fn forget(config_dir: &Path) {
    let path = chat_dir(config_dir).join("traces.db");
    let mut open = opened();
    if let Some(i) = open.iter().position(|(p, _)| *p == path) {
        let (_, conn) = open.remove(i);
        if let Ok(memory) = Connection::open_in_memory() {
            // Dropping the old connection closes the file.
            *conn.lock().unwrap_or_else(|p| p.into_inner()) = memory;
        }
    }
}

/// One connection per data folder, opened on first use.
fn connection(config_dir: &Path) -> Option<&'static Mutex<Connection>> {
    let path = chat_dir(config_dir).join("traces.db");
    let mut open = opened();
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

mod attribution;
mod queries;
mod recording;
mod registry;
mod signals;
mod stats;
pub use attribution::*;
pub use queries::*;
pub use recording::*;
pub use registry::*;
pub use signals::*;
pub use stats::*;

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
        signal(d, "c", Some(4), "opened_source");
        let counts = signal_counts(d, Utc::now() - chrono::Duration::days(1));
        assert_eq!(
            counts,
            vec![
                ("exported".into(), 1),
                ("opened_source".into(), 1),
                ("regenerated".into(), 2)
            ]
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
        // Made active again (a new approval) starts the idle clock afresh.
        register(d, "script_tool", "late", None, Some("chat-1"), "candidate");
        assert_eq!(artifact(d, "script_tool", "late").unwrap().last_used, None);
        register(d, "script_tool", "late", Some("h"), None, "active");
        assert!(artifact(d, "script_tool", "late")
            .unwrap()
            .last_used
            .is_some());
    }
}
