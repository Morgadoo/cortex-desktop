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

use super::traces;

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
    let tools = super::script_tools::list(config_dir);
    let mut week = Vec::new();
    let mut fortnight = Vec::new();
    for tool in tools
        .iter()
        .filter(|t| t.status == super::script_tools::Status::Active)
    {
        let name = format!("{}__{}", super::script_tools::CONNECTOR, tool.name);
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
                let _ = super::script_tools::retire(config_dir, &id);
            }
            "skill" => {
                let _ = super::skills::set_enabled(config_dir, &id, false);
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
        let Some(tool) = super::script_tools::load(config_dir, &name)
            .filter(|t| t.status == super::script_tools::Status::Active)
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
        if super::script_tools::revert(config_dir, &name, &tool.checks).await == Ok(true) {
            if let Some(mut back) = super::script_tools::load(config_dir, &name) {
                back.monitor_reverted_at = Some(now);
                let _ = super::script_tools::save(config_dir, &back);
            }
            report.reverted.push(format!("script_tool:{name}"));
        } else if super::script_tools::demote(config_dir, &name).is_ok() {
            report.demoted.push(format!("script_tool:{name}"));
        }
    }
    for name in decisions.demote {
        let id = format!("script_tool:{name}");
        if report.reverted.contains(&id) || report.demoted.contains(&id) {
            continue;
        }
        // Only a tool in use is demoted; a retired one stays retired.
        if !super::script_tools::load(config_dir, &name)
            .is_some_and(|t| t.status == super::script_tools::Status::Active)
        {
            continue;
        }
        if super::script_tools::demote(config_dir, &name).is_ok() {
            report.demoted.push(format!("script_tool:{name}"));
        }
    }
    // consolidate(scope): duplicate facts merged, scope by scope.
    report.consolidated = super::memory::consolidate(config_dir, None).unwrap_or(0);
    report
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    last_run: Option<DateTime<Utc>>,
}

fn state_path(config_dir: &Path) -> std::path::PathBuf {
    super::store::chat_dir(config_dir).join("lifecycle.json")
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
    let _ = super::store::write_json(
        &path,
        &State {
            last_run: Some(now),
        },
    );
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn unused_assistant_tools_retire_and_the_user_s_and_critical_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let script = "function main() { return 1; }";
        for (name, description) in [
            ("old_tool", "Converts temperatures between scales."),
            ("kept_tool", "Formats invoice numbers."),
        ] {
            super::super::script_tools::create(
                d,
                "chat-1",
                &json!({ "name": name, "description": description, "script": script }),
            )
            .await
            .unwrap();
            super::super::script_tools::promote(d, name, None)
                .await
                .unwrap();
        }
        traces::set_critical(d, "script_tool", "kept_tool", true);
        // Something the user made, used once: never touched.
        traces::used(d, "skill", "my-own-skill");

        let soon = maintain(d, Utc::now() + Duration::days(1)).await;
        assert!(soon.is_empty(), "{soon:?}");

        let later = maintain(d, Utc::now() + Duration::days(UNUSED_DAYS + 1)).await;
        assert_eq!(later.deprecated, vec!["script_tool:old_tool".to_string()]);
        assert!(super::super::script_tools::definitions(d)
            .iter()
            .all(|t| t.tool_name != "old_tool"));
        assert_eq!(
            traces::artifact(d, "skill", "my-own-skill").unwrap().status,
            "active"
        );

        let much_later = maintain(d, Utc::now() + Duration::days(ARCHIVE_DAYS + 1)).await;
        assert_eq!(
            much_later.archived,
            vec!["script_tool:old_tool".to_string()]
        );
    }

    #[tokio::test]
    async fn a_failing_tool_is_demoted() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        super::super::script_tools::create(
            d,
            "",
            &json!({ "name": "flaky", "description": "x", "script": "function main() { return 1; }" }),
        )
        .await
        .unwrap();
        super::super::script_tools::promote(d, "flaky", None)
            .await
            .unwrap();
        traces::start_run(d, "r", "c", None, "m");
        for ok in [false, false, false, false, true] {
            traces::tool_call(d, "r", "script__flaky", 5, ok);
        }
        let report = maintain(d, Utc::now()).await;
        assert_eq!(report.demoted, vec!["script_tool:flaky".to_string()]);
        assert!(super::super::script_tools::definitions(d).is_empty());
    }

    /// A tool the user just approved again is judged on its own calls: the failures of the version
    /// it replaced do not demote it.
    #[tokio::test]
    async fn a_fresh_approval_is_judged_on_its_own_calls() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        st::create(d, "chat-1", &json!({ "name": "fixed_tool", "description": "Fixed.", "script": "function main() { return 1; }" }))
            .await
            .unwrap();
        traces::start_run(d, "r", "c", None, "m");
        // The broken version failed five of six calls yesterday...
        for ok in [false, false, false, false, false, true] {
            traces::tool_call_at(
                d,
                "r",
                "script__fixed_tool",
                Utc::now() - Duration::days(1),
                5,
                ok,
            );
        }
        // ...and the user approves the fixed one today.
        st::promote(d, "fixed_tool", None).await.unwrap();
        let report = maintain(d, Utc::now()).await;
        assert!(
            report.demoted.is_empty() && report.reverted.is_empty(),
            "{report:?}"
        );
        assert_eq!(
            st::load(d, "fixed_tool").unwrap().status,
            st::Status::Active
        );
    }

    /// A version brought back from History is judged on its own calls from then on: the failures
    /// of the version it replaced do not demote it the next day.
    #[tokio::test]
    async fn a_version_restored_from_history_is_judged_on_its_own_calls() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for script in [
            "function main() { return 1; }",
            "function main() { return 2; }",
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "rates_h", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "rates_h", None).await.unwrap();
            let mut tool = st::load(d, "rates_h").unwrap();
            tool.checks.clear();
            tool.approved_at = Some(Utc::now() - Duration::days(10));
            st::save(d, &tool).unwrap();
        }
        traces::start_run(d, "r", "c", None, "m");
        // The second version failed five of six calls over the last two days...
        for ok in [false, false, false, false, false, true] {
            traces::tool_call_at(
                d,
                "r",
                "script__rates_h",
                Utc::now() - Duration::days(1),
                5,
                ok,
            );
        }
        // ...and the user brings the first back from History.
        let first = super::super::versions::list(d, "tools", "script-rates_h")
            .unwrap()
            .into_iter()
            .find(|v| {
                v.value["script"] == "function main() { return 1; }"
                    && v.value["status"] == "active"
            })
            .unwrap();
        super::super::versions::restore(d, "tools", "script-rates_h", &first.id).unwrap();
        assert!(st::load(d, "rates_h").unwrap().script.contains("return 1"));
        let report = maintain(d, Utc::now() + Duration::hours(1)).await;
        assert!(
            report.demoted.is_empty() && report.reverted.is_empty(),
            "{report:?}"
        );
        assert_eq!(st::load(d, "rates_h").unwrap().status, st::Status::Active);
    }

    /// Until a restored version has a week of its own calls, it is not compared with the weeks of
    /// the versions the user set aside: a clean week of theirs is no baseline for it.
    #[tokio::test]
    async fn a_restored_version_is_not_judged_against_the_weeks_it_set_aside() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let now = Utc::now();
        for (script, approved) in [
            ("function main() { return 1; }", now - Duration::days(30)),
            ("function main() { return 2; }", now - Duration::days(20)),
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "rates_b", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "rates_b", None).await.unwrap();
            let mut tool = st::load(d, "rates_b").unwrap();
            tool.checks.clear();
            tool.approved_at = Some(approved);
            st::save(d, &tool).unwrap();
        }
        // The second version ran clean all week...
        traces::start_run(d, "r", "c", None, "m");
        for _ in 0..10 {
            traces::tool_call_at(d, "r", "script__rates_b", now - Duration::days(4), 5, true);
        }
        // ...and the user still brings the first back from History.
        let first = super::super::versions::list(d, "tools", "script-rates_b")
            .unwrap()
            .into_iter()
            .find(|v| {
                v.value["script"] == "function main() { return 1; }"
                    && v.value["status"] == "active"
            })
            .unwrap();
        super::super::versions::restore(d, "tools", "script-rates_b", &first.id).unwrap();
        // Two of its first six calls fail: worse than the set-aside version's week, but that is
        // the user's choice, not a regression.
        for ok in [false, false, true, true, true, true] {
            traces::tool_call_at(
                d,
                "r",
                "script__rates_b",
                Utc::now() + Duration::minutes(30),
                5,
                ok,
            );
        }
        let report = maintain(d, Utc::now() + Duration::hours(2)).await;
        assert!(
            report.reverted.is_empty() && report.demoted.is_empty(),
            "{report:?}"
        );
        let tool = st::load(d, "rates_b").unwrap();
        assert!(tool.script.contains("return 1"));
        assert_eq!(tool.status, st::Status::Active);
    }

    /// Restoring a version from History sets aside every version approved after it, not only the
    /// current one: the monitor never rolls forward to any of them. A rewrite keeps that list, and
    /// bringing back a set-aside version takes it off.
    #[tokio::test]
    async fn restoring_sets_aside_every_later_version() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let now = Utc::now();
        for (script, approved) in [
            ("function main() { return 1; }", now - Duration::days(60)),
            ("function main() { return 2; }", now - Duration::days(50)),
            ("function main() { return 3; }", now - Duration::days(40)),
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "rates_i", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "rates_i", None).await.unwrap();
            let mut tool = st::load(d, "rates_i").unwrap();
            tool.checks.clear();
            tool.approved_at = Some(approved);
            st::save(d, &tool).unwrap();
        }
        let active = |script: &str| {
            super::super::versions::list(d, "tools", "script-rates_i")
                .unwrap()
                .into_iter()
                .find(|v| v.value["script"] == script && v.value["status"] == "active")
                .unwrap()
        };
        let first = active("function main() { return 1; }");
        super::super::versions::restore(d, "tools", "script-rates_i", &first.id).unwrap();
        assert_eq!(st::load(d, "rates_i").unwrap().set_aside.len(), 2);
        // Weeks later the first version has a bad week against its own earlier one: it is not
        // rolled forward to the second version, the one before the current at the time.
        let mut tool = st::load(d, "rates_i").unwrap();
        tool.approved_at = Some(now - Duration::days(30));
        tool.restored_at = Some(now - Duration::days(30));
        st::save(d, &tool).unwrap();
        traces::start_run(d, "r", "c", None, "m");
        for i in 0..10 {
            traces::tool_call_at(d, "r", "script__rates_i", now - Duration::days(10), 5, true);
            traces::tool_call_at(
                d,
                "r",
                "script__rates_i",
                now - Duration::days(1),
                5,
                i >= 6,
            );
        }
        let report = maintain(d, now).await;
        assert!(report.reverted.is_empty(), "{report:?}");
        assert!(st::load(d, "rates_i").unwrap().script.contains("return 1"));

        // A rewrite keeps the list.
        let mut tool = st::load(d, "rates_i").unwrap();
        tool.status = st::Status::Active;
        st::save(d, &tool).unwrap();
        let before = st::load(d, "rates_i").unwrap().set_aside;
        st::create(
            d,
            "chat-1",
            &json!({ "name": "rates_i", "description": "Rates.", "script": "function main() { return 4; }" }),
        )
        .await
        .unwrap();
        assert_eq!(st::load(d, "rates_i").unwrap().set_aside, before);

        // Bringing back the third version takes it off the list.
        let third = active("function main() { return 3; }");
        let third_hash = st::approval_hash(&serde_json::from_value(third.value.clone()).unwrap());
        super::super::versions::restore(d, "tools", "script-rates_i", &third.id).unwrap();
        let back = st::load(d, "rates_i").unwrap();
        assert!(back.script.contains("return 3"));
        assert!(
            !back.set_aside.contains(&third_hash),
            "{:?}",
            back.set_aside
        );
    }

    /// A freshly promoted version that does worse than the one it replaced goes back to it.
    #[tokio::test]
    async fn a_fresh_promotion_that_does_worse_goes_back() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let now = Utc::now();
        for (script, approved) in [
            ("function main() { return 1; }", now - Duration::days(30)),
            ("function main() { return 2; }", now - Duration::days(6)),
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "rates_p", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "rates_p", None).await.unwrap();
            let mut tool = st::load(d, "rates_p").unwrap();
            tool.checks.clear();
            tool.approved_at = Some(approved);
            st::save(d, &tool).unwrap();
        }
        traces::start_run(d, "r", "c", None, "m");
        // The first version ran clean the week before; the new one fails nine of twenty.
        for i in 0..20 {
            traces::tool_call_at(d, "r", "script__rates_p", now - Duration::days(10), 5, true);
            traces::tool_call_at(
                d,
                "r",
                "script__rates_p",
                now - Duration::days(2),
                5,
                i >= 9,
            );
        }
        let report = maintain(d, now).await;
        assert_eq!(
            report.reverted,
            vec!["script_tool:rates_p".to_string()],
            "{report:?}"
        );
        assert!(st::load(d, "rates_p").unwrap().script.contains("return 1"));
    }

    /// A version the user set aside by restoring an earlier one from History is never rolled
    /// forward to, however long after.
    #[tokio::test]
    async fn the_monitor_never_rolls_forward_to_a_version_the_user_set_aside() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for script in [
            "function main() { return 1; }",
            "function main() { return 2; }",
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "rates_f", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "rates_f", None).await.unwrap();
            let mut tool = st::load(d, "rates_f").unwrap();
            tool.checks.clear();
            st::save(d, &tool).unwrap();
        }
        // The user brings back the first version from History, setting the second aside.
        let first = super::super::versions::list(d, "tools", "script-rates_f")
            .unwrap()
            .into_iter()
            .find(|v| {
                v.value["script"] == "function main() { return 1; }"
                    && v.value["status"] == "active"
            })
            .unwrap();
        super::super::versions::restore(d, "tools", "script-rates_f", &first.id).unwrap();
        assert_eq!(st::load(d, "rates_f").unwrap().set_aside.len(), 1);
        // Weeks later the first version has a bad week against its own earlier one.
        let mut tool = st::load(d, "rates_f").unwrap();
        tool.approved_at = Some(Utc::now() - Duration::days(30));
        st::save(d, &tool).unwrap();
        let now = Utc::now();
        traces::start_run(d, "r", "c", None, "m");
        for i in 0..10 {
            traces::tool_call_at(d, "r", "script__rates_f", now - Duration::days(10), 5, true);
            traces::tool_call_at(
                d,
                "r",
                "script__rates_f",
                now - Duration::days(1),
                5,
                i >= 6,
            );
        }
        let report = maintain(d, now).await;
        assert!(report.reverted.is_empty(), "{report:?}");
        assert!(
            st::load(d, "rates_f").unwrap().script.contains("return 1"),
            "not the set-aside one"
        );
    }

    /// A tool the assistant wrote and keeps calling is used, so it is not retired.
    #[tokio::test]
    async fn a_tool_in_use_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        super::super::script_tools::create(
            d,
            "chat-1",
            &json!({ "name": "daily_tool", "description": "Formats dates.", "script": "function main() { return 1; }" }),
        )
        .await
        .unwrap();
        super::super::script_tools::promote(d, "daily_tool", None)
            .await
            .unwrap();
        let later = Utc::now() + Duration::days(UNUSED_DAYS + 10);
        traces::start_run(d, "r", "c", None, "m");
        traces::tool_call_at(
            d,
            "r",
            "script__daily_tool",
            later - Duration::days(2),
            5,
            true,
        );
        let report = maintain(d, later).await;
        assert!(report.deprecated.is_empty(), "{report:?}");
        assert_eq!(
            traces::artifact(d, "script_tool", "daily_tool")
                .unwrap()
                .used,
            1
        );
        assert_eq!(super::super::script_tools::definitions(d).len(), 1);
    }

    /// Monitor: failing more this week than last sends a tool back to its previous approved
    /// version; with none, back to candidate.
    #[tokio::test]
    async fn a_tool_that_got_worse_goes_back_a_version_or_out() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let write = |name: &'static str, script: &'static str| async move {
            st::create(d, "chat-1", &json!({ "name": name, "description": format!("{name} helper."), "script": script }))
                .await
                .unwrap();
            st::promote(d, name, None).await.unwrap();
            let mut tool = st::load(d, name).unwrap();
            tool.checks.clear();
            // Approved a while ago: the weeks below are its own.
            tool.approved_at = Some(Utc::now() - Duration::days(20));
            st::save(d, &tool).unwrap();
        };
        write("rates_v", "function main() { return 1; }").await;
        write("rates_v", "function main() { return 2; }").await;
        write("lonely", "function main() { return 3; }").await;
        let now = Utc::now();
        traces::start_run(d, "r", "c", None, "m");
        for tool in ["script__rates_v", "script__lonely"] {
            // Last week: ten calls, none failed; this week: ten calls, six failed: past the demotion
            // threshold too, so the next day's pass must not demote the version rolled back to.
            for i in 0..10 {
                traces::tool_call_at(d, "r", tool, now - Duration::days(10), 5, true);
                traces::tool_call_at(d, "r", tool, now - Duration::days(1), 5, i >= 6);
            }
        }
        let report = maintain(d, now).await;
        assert_eq!(report.reverted, vec!["script_tool:rates_v".to_string()]);
        assert_eq!(report.demoted, vec!["script_tool:lonely".to_string()]);
        assert!(st::load(d, "rates_v").unwrap().script.contains("return 1"));
        assert_eq!(st::load(d, "lonely").unwrap().status, st::Status::Candidate);
        // The next day the same week's failures do not flip it back to the broken version.
        let next = maintain(d, now + Duration::days(1)).await;
        assert!(next.reverted.is_empty(), "{next:?}");
        // Nor demote the version it went back to, for the failures of the one it replaced.
        assert!(
            !next.demoted.contains(&"script_tool:rates_v".to_string()),
            "{next:?}"
        );
        let back = st::load(d, "rates_v").unwrap();
        assert!(back.script.contains("return 1"));
        assert_eq!(back.status, st::Status::Active);
    }

    /// A retired tool is not brought back by the monitor, however its last week went.
    #[tokio::test]
    async fn the_monitor_leaves_a_retired_tool_retired() {
        use super::super::script_tools as st;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for script in [
            "function main() { return 1; }",
            "function main() { return 2; }",
        ] {
            st::create(
                d,
                "chat-1",
                &json!({ "name": "old_rates", "description": "Rates.", "script": script }),
            )
            .await
            .unwrap();
            st::promote(d, "old_rates", None).await.unwrap();
            let mut tool = st::load(d, "old_rates").unwrap();
            tool.checks.clear();
            st::save(d, &tool).unwrap();
        }
        st::retire(d, "old_rates").unwrap();
        let now = Utc::now();
        traces::start_run(d, "r", "c", None, "m");
        for i in 0..10 {
            traces::tool_call_at(
                d,
                "r",
                "script__old_rates",
                now - Duration::days(10),
                5,
                true,
            );
            traces::tool_call_at(
                d,
                "r",
                "script__old_rates",
                now - Duration::days(1),
                5,
                i >= 4,
            );
        }
        let report = maintain(d, now).await;
        assert!(report.reverted.is_empty(), "{report:?}");
        assert_eq!(
            st::load(d, "old_rates").unwrap().status,
            st::Status::Deprecated
        );
    }

    #[tokio::test]
    async fn it_runs_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        assert!(daily(dir.path(), now).await.is_some());
        assert!(daily(dir.path(), now + Duration::hours(2)).await.is_none());
        assert!(daily(dir.path(), now + Duration::hours(25)).await.is_some());
    }
}
