//! What a run had and used (skills, tools, facts, pages), and the feedback it got.

use super::*;

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

pub(super) fn ensure(c: &Connection, kind: &str, id: &str) -> rusqlite::Result<usize> {
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

pub(super) fn used_at(config_dir: &Path, kind: &str, id: &str, at: i64) {
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
