//! The activity log (OpenBot's audit log): every tool call the assistant made and every approval
//! decision, with when, in which chat, for which agent, and how it ended. Kept on this computer
//! (`chat/audit/<year-month>.jsonl`, encrypted at rest when that is on), for a year, and exported
//! as CSV or JSON from Settings → Profile.
//!
//! Arguments and results are kept short (a few hundred characters) and secrets from the vault
//! are already masked before they reach here.

use crate::support::error::StoreError;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Characters kept of a call's arguments and result.
const KEEP: usize = 300;
/// Entries kept per month.
const MONTH_LIMIT: usize = 50_000;
/// Months kept.
const MONTHS: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: DateTime<Utc>,
    /// `tool` (a call ran) or `approval` (a decision was made).
    pub kind: String,
    pub conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub arguments: String,
    /// `ok`, `error`, or for approvals `approved`, `approved always`, `denied`.
    pub outcome: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub millis: u64,
}

fn dir(config_dir: &Path) -> PathBuf {
    crate::chat::store::chat_dir(config_dir).join("audit")
}

fn month_file(config_dir: &Path, at: DateTime<Utc>) -> PathBuf {
    dir(config_dir).join(format!("{}.jsonl", at.format("%Y-%m")))
}

pub fn short(text: &str) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > KEEP {
        format!("{}…", crate::support::text::clip(&t, KEEP))
    } else {
        t
    }
}

fn lock() -> &'static std::sync::Mutex<()> {
    static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| std::sync::Mutex::new(()))
}

/// Adds an entry. Failing to log never stops a call: it is reported in the app's log instead.
pub fn record(config_dir: &Path, entry: Entry) {
    let _guard = lock().lock();
    let path = month_file(config_dir, entry.at);
    let result = (|| -> Result<(), StoreError> {
        std::fs::create_dir_all(dir(config_dir))
            .map_err(|e| StoreError::write(&dir(config_dir), e))?;
        let mut text = crate::chat::at_rest::read_to_string(&path).unwrap_or_default();
        if text.lines().count() >= MONTH_LIMIT {
            return Ok(());
        }
        text.push_str(
            &serde_json::to_string(&entry).map_err(|source| StoreError::Encode {
                path: path.clone(),
                source,
            })?,
        );
        text.push('\n');
        crate::chat::at_rest::write(&path, text).map_err(|e| StoreError::write(&path, e))?;
        // Older than a year: gone.
        let mut months: Vec<PathBuf> = std::fs::read_dir(dir(config_dir))
            .map_err(|e| StoreError::read(&dir(config_dir), e))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        months.sort();
        while months.len() > MONTHS {
            let _ = std::fs::remove_file(months.remove(0));
        }
        Ok(())
    })();
    if let Err(e) = result {
        log::warn!("could not add to the activity log: {e}");
    }
}

/// Entries since `since` (all when `None`), oldest first.
pub fn entries(config_dir: &Path, since: Option<DateTime<Utc>>) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir(config_dir)) else {
        return Vec::new();
    };
    let mut months: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    months.sort();
    months
        .into_iter()
        .flat_map(|p| {
            crate::chat::at_rest::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
                .collect::<Vec<_>>()
        })
        .filter(|e| since.is_none_or(|s| e.at >= s))
        .collect()
}

pub fn clear(config_dir: &Path) -> Result<(), crate::support::error::StoreError> {
    match std::fs::remove_dir_all(dir(config_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(crate::support::error::StoreError::write(
            &dir(config_dir),
            e,
        )),
    }
}

fn csv_cell(s: &str) -> String {
    // A leading = + - @ would run as a formula in a spreadsheet.
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn to_csv(entries: &[Entry]) -> String {
    let mut out =
        String::from("time,kind,conversation,agent,tool,arguments,outcome,detail,milliseconds\n");
    for e in entries {
        let row = [
            e.at.to_rfc3339(),
            e.kind.clone(),
            e.conversation.clone(),
            e.agent.clone().unwrap_or_default(),
            e.tool.clone(),
            e.arguments.clone(),
            e.outcome.clone(),
            e.detail.clone(),
            e.millis.to_string(),
        ];
        out.push_str(
            &row.iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, outcome: &str) -> Entry {
        Entry {
            at: Utc::now(),
            kind: "tool".into(),
            conversation: "c1".into(),
            agent: None,
            tool: tool.into(),
            arguments: short(&"x ".repeat(400)),
            outcome: outcome.into(),
            detail: String::new(),
            millis: 12,
        }
    }

    #[test]
    fn entries_are_kept_read_and_exported() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), entry("web_search", "ok"));
        record(
            dir.path(),
            Entry {
                kind: "approval".into(),
                detail: "=SUM(A1)".into(),
                ..entry("filesystem__write_file", "denied")
            },
        );
        let all = entries(dir.path(), None);
        assert_eq!(all.len(), 2);
        assert!(all[0].arguments.chars().count() <= KEEP + 1);
        assert!(entries(dir.path(), Some(Utc::now() + chrono::Duration::minutes(1))).is_empty());
        let csv = to_csv(&all);
        assert!(csv.starts_with("time,kind,"));
        assert!(csv.contains(",approval,") && csv.contains("'=SUM(A1)"));
        clear(dir.path()).unwrap();
        assert!(entries(dir.path(), None).is_empty());
    }
}
