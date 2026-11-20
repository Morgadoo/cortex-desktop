//! Implicit signals about an answer (copied, edited, regenerated) and ratings.

use super::*;

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
