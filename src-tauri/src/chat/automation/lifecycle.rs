//! The lifecycle of what the assistant makes for itself (skills, script tools): what is not used
//! is retired, and a tool that keeps failing goes back to being a candidate.
//!
//! It runs once a day from the scheduler. It only touches what the assistant made: an artifact
//! whose provenance is unknown counts as the user's and is left alone, and so is one the user
//! marked critical, however rarely it is used. Nothing is deleted: a deprecated artifact is
//! hidden and restorable, an archived one is kept on disk.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::chat::traces;

use cortex_kernel::lifecycle::decide;
#[cfg(test)]
use cortex_kernel::lifecycle::{ARCHIVE_DAYS, UNUSED_DAYS};

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub deprecated: Vec<String>,
    pub archived: Vec<String>,
    pub demoted: Vec<String>,
    /// Script tools that failed more this week than last, back on their previous version.
    pub reverted: Vec<String>,
    /// Duplicate facts merged.
    pub consolidated: usize,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.deprecated.is_empty()
            && self.archived.is_empty()
            && self.demoted.is_empty()
            && self.reverted.is_empty()
            && self.consolidated == 0
    }
}

/// One pass of the lifecycle, as of `now`: the kernel decides, this applies.
pub async fn maintain(config_dir: &Path, now: DateTime<Utc>) -> Report {
    let artifacts = traces::artifacts(config_dir, None);
    // Each tool in use is judged on its calls since the user last approved it, within the week:
    // failures of a version the user has since replaced do not count against the new one.
    let tools = crate::chat::script_tools::list(config_dir);
    let mut week = Vec::new();
    let mut fortnight = Vec::new();
    for tool in tools
        .iter()
        .filter(|t| t.status == crate::chat::script_tools::Status::Active)
    {
        let name = format!("{}__{}", crate::chat::script_tools::CONNECTOR, tool.name);
        let start = tool.approved_at.map_or(now - Duration::days(7), |at| {
            at.max(now - Duration::days(7))
        });
        week.push(traces::tool_stat(
            config_dir,
            &name,
            start,
            now + Duration::seconds(1),
        ));
        fortnight.push(traces::tool_stat(
            config_dir,
            &name,
            start - Duration::days(7),
            now + Duration::seconds(1),
        ));
    }
    let decisions = decide(&artifacts, &week, now);
    let mut report = Report::default();
    for (kind, id) in decisions.deprecate {
        match kind.as_str() {
            "script_tool" => {
                let _ = crate::chat::script_tools::retire(config_dir, &id);
            }
            "skill" => {
                let _ = crate::chat::skills::set_enabled(config_dir, &id, false);
            }
            _ => {}
        }
        traces::set_status(config_dir, &kind, &id, "deprecated");
        report.deprecated.push(format!("{kind}:{id}"));
    }
    for (kind, id) in decisions.archive {
        traces::set_status(config_dir, &kind, &id, "archived");
        report.archived.push(format!("{kind}:{id}"));
    }
    // Monitor: a tool that fails more this week than last goes back to its previous version, or,
    // with no earlier version that passes its checks, back to candidate.
    for name in cortex_kernel::lifecycle::regressed(&week, &fortnight) {
        // Only a tool in use is monitored: a retired or candidate one is not brought back.
        let Some(tool) = crate::chat::script_tools::load(config_dir, &name)
            .filter(|t| t.status == crate::chat::script_tools::Status::Active)
        else {
            continue;
        };
        // A version the user brought back from History is compared week over week only once it
        // has a week of its own calls as the baseline; until then "last week" is the versions the
        // user set aside. The failure-rate demotion still applies.
        if tool
            .restored_at
            .is_some_and(|at| at > now - Duration::days(14))
        {
            continue;
        }
        // The week that made it roll back still holds the failures that did; judging it on them
        // again would flip it back and forth. It is judged again once a new week has passed.
        if tool
            .monitor_reverted_at
            .is_some_and(|at| now - at < Duration::days(7))
        {
            continue;
        }
        if crate::chat::script_tools::revert(config_dir, &name, &tool.checks).await == Ok(true) {
            if let Some(mut back) = crate::chat::script_tools::load(config_dir, &name) {
                back.monitor_reverted_at = Some(now);
                if let Err(e) = crate::chat::script_tools::save(config_dir, &back) {
                    log::warn!("could not record that {name} was reverted: {e}");
                }
            }
            report.reverted.push(format!("script_tool:{name}"));
        } else if crate::chat::script_tools::demote(config_dir, &name).is_ok() {
            report.demoted.push(format!("script_tool:{name}"));
        }
    }
    for name in decisions.demote {
        let id = format!("script_tool:{name}");
        if report.reverted.contains(&id) || report.demoted.contains(&id) {
            continue;
        }
        // Only a tool in use is demoted; a retired one stays retired.
        if !crate::chat::script_tools::load(config_dir, &name)
            .is_some_and(|t| t.status == crate::chat::script_tools::Status::Active)
        {
            continue;
        }
        if crate::chat::script_tools::demote(config_dir, &name).is_ok() {
            report.demoted.push(format!("script_tool:{name}"));
        }
    }
    // consolidate(scope): duplicate facts merged, scope by scope.
    report.consolidated = crate::chat::memory::consolidate(config_dir, None).unwrap_or(0);
    report
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    last_run: Option<DateTime<Utc>>,
}

fn state_path(config_dir: &Path) -> std::path::PathBuf {
    crate::chat::store::chat_dir(config_dir).join("lifecycle.json")
}

/// Runs [`maintain`] at most once a day; returns its report when it ran.
pub async fn daily(config_dir: &Path, now: DateTime<Utc>) -> Option<Report> {
    let path = state_path(config_dir);
    let state: State = crate::chat::at_rest::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if state.last_run.is_some_and(|t| now - t < Duration::days(1)) {
        return None;
    }
    let report = maintain(config_dir, now).await;
    if let Err(e) = crate::chat::store::write_json(
        &path,
        &State {
            last_run: Some(now),
        },
    ) {
        log::warn!("could not record the maintenance run: {e}");
    }
    Some(report)
}

#[cfg(test)]
mod tests;
