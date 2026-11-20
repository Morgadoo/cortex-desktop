//! The artifact registry: what the assistant made for itself, its versions, status and whether it is critical.

use super::*;

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
            // Made active (approved, promoted again) is a fresh start for the lifecycle's idle
            // clock, so a tool approved after a long wait is not retired the next day.
            "UPDATE artifacts SET version = CASE WHEN hash IS NULL OR hash = ?3 THEN version ELSE version + 1 END, hash = ?3, provenance = COALESCE(?4, provenance), status = ?5, last_used = CASE WHEN ?5 = 'active' THEN MAX(COALESCE(last_used, 0), ?6) ELSE last_used END WHERE kind = ?1 AND id = ?2",
            params![kind, id, hash, provenance, status, now()],
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

pub(super) fn time(ms: i64) -> DateTime<Utc> {
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

pub(super) fn row_artifact(r: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
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
