//! Earlier versions of what the user writes and tunes: agents, workflows and their own tools, as
//! skills already keep (LobeHub and Open WebUI keep versions of prompts and assistants).
//!
//! Saving a changed item first keeps the version it replaces, as JSON in
//! `chat/versions/<kind>/<id>/<time>.json`; the newest [`KEEP`] per item stay. Restoring is
//! saving an old version again, so the one it replaces is kept too.
//!
//! One version of an item can be marked production (LibreChat's prompt versions): that one is what
//! runs, and saved edits stay a draft until they are marked in turn. It is kept apart, in
//! `chat/versions/<kind>/<id>.production.json`, so pruning never takes it.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::chat_dir;

/// Versions kept per item.
const KEEP: usize = 30;

pub const KINDS: &[&str] = &["agents", "workflows", "tools"];

fn dir(config_dir: &Path, kind: &str, id: &str) -> Result<PathBuf, String> {
    if !KINDS.contains(&kind) {
        return Err(format!("Unknown kind {kind}."));
    }
    if id.is_empty() || id.len() > 80 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("invalid id".into());
    }
    Ok(chat_dir(config_dir).join("versions").join(kind).join(id))
}

/// Keeps `previous` as a version of the item, when it differs from what is about to be saved.
pub fn keep<T: Serialize>(config_dir: &Path, kind: &str, id: &str, previous: &T, next: &T) -> Result<(), String> {
    let (previous, next) = (serde_json::to_value(previous).map_err(|e| e.to_string())?, serde_json::to_value(next).map_err(|e| e.to_string())?);
    if previous == next {
        return Ok(());
    }
    let dir = dir(config_dir, kind, id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let mut file = dir.join(format!("{stamp}.json"));
    let mut n = 1;
    while file.exists() {
        file = dir.join(format!("{stamp}-{n}.json"));
        n += 1;
    }
    crate::chat::at_rest::write(&file, serde_json::to_string_pretty(&previous).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut files: Vec<_> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    for old in files.iter().take(files.len().saturating_sub(KEEP)) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// When it was replaced (sortable).
    pub id: String,
    pub value: Value,
}

/// An item's earlier versions, newest first.
pub fn list(config_dir: &Path, kind: &str, id: &str) -> Result<Vec<Version>, String> {
    let dir = dir(config_dir, kind, id)?;
    let mut out: Vec<Version> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.path().file_stem()?.to_string_lossy().into_owned();
            let value = serde_json::from_str(&crate::chat::at_rest::read_to_string(e.path()).ok()?).ok()?;
            Some(Version { id, value })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// The version marked production, and when it was marked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Production {
    pub at: DateTime<Utc>,
    pub value: Value,
}

fn production_path(config_dir: &Path, kind: &str, id: &str) -> Result<PathBuf, String> {
    let dir = dir(config_dir, kind, id)?;
    Ok(dir.with_file_name(format!("{id}.production.json")))
}

pub fn production(config_dir: &Path, kind: &str, id: &str) -> Option<Production> {
    let text = crate::chat::at_rest::read_to_string(production_path(config_dir, kind, id).ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Marks `value` as the item's production version, or, with `None`, goes back to running the
/// current one.
pub fn set_production(config_dir: &Path, kind: &str, id: &str, value: Option<Value>) -> Result<Option<Production>, String> {
    let path = production_path(config_dir, kind, id)?;
    match value {
        Some(value) => {
            let marked = Production { at: Utc::now(), value };
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            super::store::write_json(&path, &marked)?;
            Ok(Some(marked))
        }
        None => {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_version_can_be_production() {
        let dir = tempfile::tempdir().unwrap();
        assert!(production(dir.path(), "workflows", "w1").is_none());
        set_production(dir.path(), "workflows", "w1", Some(json!({"steps": ["a"]}))).unwrap();
        assert_eq!(production(dir.path(), "workflows", "w1").unwrap().value, json!({"steps": ["a"]}));
        for i in 0..40 {
            keep(dir.path(), "workflows", "w1", &json!(i), &json!(i + 1)).unwrap();
        }
        assert_eq!(list(dir.path(), "workflows", "w1").unwrap().len(), KEEP, "the marker is not a version");
        assert!(production(dir.path(), "workflows", "w1").is_some(), "pruning keeps it");
        set_production(dir.path(), "workflows", "w1", None).unwrap();
        set_production(dir.path(), "workflows", "w1", None).unwrap();
        assert!(production(dir.path(), "workflows", "w1").is_none());
        assert!(set_production(dir.path(), "workflows", "../x", Some(json!(1))).is_err());
    }

    #[test]
    fn changed_items_keep_what_they_replace() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = json!({ "name": "Researcher", "instructions": "Cite sources." });
        let v2 = json!({ "name": "Researcher", "instructions": "Cite sources. Be brief." });
        keep(dir.path(), "agents", "a1", &v1, &v1).unwrap();
        assert!(list(dir.path(), "agents", "a1").unwrap().is_empty(), "an unchanged save keeps nothing");
        keep(dir.path(), "agents", "a1", &v1, &v2).unwrap();
        keep(dir.path(), "agents", "a1", &v2, &v1).unwrap();
        let versions = list(dir.path(), "agents", "a1").unwrap();
        assert_eq!(versions.iter().map(|v| v.value.clone()).collect::<Vec<_>>(), [v2, v1], "newest first");
        assert!(keep(dir.path(), "secrets", "a1", &json!(1), &json!(2)).is_err());
        assert!(list(dir.path(), "agents", "../x").is_err());
        for i in 0..40 {
            keep(dir.path(), "tools", "t", &json!(i), &json!(i + 1)).unwrap();
        }
        assert_eq!(list(dir.path(), "tools", "t").unwrap().len(), KEEP);
    }
}
