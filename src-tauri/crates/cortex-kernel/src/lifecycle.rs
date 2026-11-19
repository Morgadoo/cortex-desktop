//! The lifecycle's decisions, from the artifact registry and a week of tool statistics: what to
//! retire, archive or send back to candidate. Applying them is the app's (`chat/lifecycle.rs`).
//!
//! Only what the assistant made is touched: an artifact whose provenance is unknown counts as the
//! user's, and one marked critical is kept however rarely it is used.

use chrono::{DateTime, Duration, Utc};
use cortex_traces::{Artifact, ToolStat};

/// Unused this long, an assistant-made artifact is deprecated.
pub const UNUSED_DAYS: i64 = 30;
/// Deprecated and still unused this long, it is archived.
pub const ARCHIVE_DAYS: i64 = 120;
/// A script tool with at least this many calls in a week…
pub const MIN_CALLS: i64 = 5;
/// …that fails more than this share of them is demoted.
pub const MAX_FAILURE_RATE: f64 = 0.5;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Decisions {
    /// (kind, id)
    pub deprecate: Vec<(String, String)>,
    pub archive: Vec<(String, String)>,
    /// Script tools, by name.
    pub demote: Vec<String>,
}

fn assistant_made(a: &Artifact) -> bool {
    a.provenance.as_deref().is_some_and(|p| p != "user")
}

/// How much an artifact earns its place, from 0 to 1: how often it is used when it is offered,
/// fading the longer it goes unused (half its value every 30 days).
pub fn score(a: &Artifact, now: DateTime<Utc>) -> f64 {
    let rate = (a.used as f64 + 1.0) / (a.loaded.max(a.used) as f64 + 2.0);
    let idle_days = (now - a.last_used.unwrap_or(a.created_at))
        .num_days()
        .max(0) as f64;
    rate * 0.5f64.powf(idle_days / 30.0)
}

/// A fact as consolidation sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub id: String,
    pub text: String,
    pub importance: u8,
    /// `project:<id>`, `agent:<id>`, or everywhere.
    pub scope: Option<String>,
}

/// Facts that say the same thing within the same scope, grouped: the first of each group is the
/// one to keep (the most important, then the earliest), the rest are its duplicates. Two facts say
/// the same thing when nearly all their words are shared.
pub fn duplicates(facts: &[Fact]) -> Vec<(String, Vec<String>)> {
    use std::collections::HashSet;
    let words = |t: &str| -> HashSet<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() > 2)
            .map(str::to_lowercase)
            .collect()
    };
    let sets: Vec<HashSet<String>> = facts.iter().map(|f| words(&f.text)).collect();
    // "Is allergic" and "is not allergic" share nearly every word and say the opposite: a fact
    // and its correction are never merged. Short words count here, so "no" and "not" do.
    let negated = |t: &str| -> bool {
        t.to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .any(|w| {
                matches!(
                    w,
                    "not"
                        | "no"
                        | "never"
                        | "none"
                        | "nobody"
                        | "nothing"
                        | "neither"
                        | "nor"
                        | "without"
                        | "isn't"
                        | "aren't"
                        | "wasn't"
                        | "weren't"
                        | "don't"
                        | "doesn't"
                        | "didn't"
                        | "can't"
                        | "cannot"
                        | "won't"
                        | "shouldn't"
                        | "não"
                        | "nunca"
                        | "nem"
                        | "sem"
                )
            })
    };
    let negations: Vec<bool> = facts.iter().map(|f| negated(&f.text)).collect();
    let mut taken = vec![false; facts.len()];
    let mut groups = Vec::new();
    // The most important facts lead their group.
    let mut order: Vec<usize> = (0..facts.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(facts[*i].importance));
    for &i in &order {
        if taken[i] || sets[i].is_empty() {
            continue;
        }
        let same: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&j| {
                j != i
                    && !taken[j]
                    && facts[j].scope == facts[i].scope
                    && negations[j] == negations[i]
                    && {
                        let shared = sets[i].intersection(&sets[j]).count() as f64;
                        let union = sets[i].union(&sets[j]).count().max(1) as f64;
                        shared / union >= 0.8
                    }
            })
            .collect();
        if same.is_empty() {
            continue;
        }
        taken[i] = true;
        for &j in &same {
            taken[j] = true;
        }
        groups.push((
            facts[i].id.clone(),
            same.iter().map(|&j| facts[j].id.clone()).collect(),
        ));
    }
    groups
}

/// How much worse a script tool's failure rate may get from one week to the next before it is
/// reverted to its previous version.
pub const REGRESSION: f64 = 0.25;

/// Monitor, week over week: the script tools whose failure rate this week (`week`) is at least
/// [`REGRESSION`] above the week before. `fortnight` covers both weeks, so the week before is
/// the difference. Each week needs [`MIN_CALLS`] calls for the rates to mean anything.
pub fn regressed(week: &[ToolStat], fortnight: &[ToolStat]) -> Vec<String> {
    let rate = |calls: i64, failures: i64| failures as f64 / calls.max(1) as f64;
    week.iter()
        .filter_map(|now| {
            let name = now.tool.strip_prefix("script__")?;
            let both = fortnight.iter().find(|s| s.tool == now.tool)?;
            let (calls, failures) = (both.calls - now.calls, both.failures - now.failures);
            (now.calls >= MIN_CALLS
                && calls >= MIN_CALLS
                && rate(now.calls, now.failures) >= rate(calls, failures) + REGRESSION)
                .then(|| name.to_string())
        })
        .collect()
}

pub fn decide(artifacts: &[Artifact], week: &[ToolStat], now: DateTime<Utc>) -> Decisions {
    let mut d = Decisions::default();
    for a in artifacts {
        if !matches!(a.kind.as_str(), "skill" | "script_tool") || a.critical || !assistant_made(a) {
            continue;
        }
        let idle = now - a.last_used.unwrap_or(a.created_at);
        match a.status.as_str() {
            "active" if idle > Duration::days(UNUSED_DAYS) => {
                d.deprecate.push((a.kind.clone(), a.id.clone()))
            }
            // Deprecated, or written and never approved, and idle this long: archived.
            "deprecated" | "candidate" if idle > Duration::days(ARCHIVE_DAYS) => {
                d.archive.push((a.kind.clone(), a.id.clone()))
            }
            _ => {}
        }
    }
    for stat in week {
        if let Some(name) = stat.tool.strip_prefix("script__") {
            if stat.calls >= MIN_CALLS
                && stat.failures as f64 / stat.calls as f64 > MAX_FAILURE_RATE
            {
                d.demote.push(name.to_string());
            }
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(
        kind: &str,
        status: &str,
        provenance: Option<&str>,
        idle_days: i64,
        critical: bool,
    ) -> Artifact {
        let at = Utc::now() - Duration::days(idle_days);
        Artifact {
            kind: kind.into(),
            id: format!("{kind}-{status}-{idle_days}"),
            status: status.into(),
            version: 1,
            hash: None,
            provenance: provenance.map(str::to_string),
            critical,
            loaded: 0,
            used: 0,
            created_at: at,
            last_used: None,
        }
    }

    #[test]
    fn facts_that_say_the_same_thing_in_the_same_scope_are_grouped() {
        let fact = |id: &str, text: &str, importance: u8, scope: Option<&str>| Fact {
            id: id.into(),
            text: text.into(),
            importance,
            scope: scope.map(str::to_string),
        };
        let facts = [
            fact("a", "Prefers answers in metric units", 2, None),
            fact("b", "prefers answers in metric units.", 3, None),
            fact("c", "Prefers answers in metric units", 2, Some("project:x")),
            fact("d", "Lives in Lisbon", 2, None),
        ];
        // b is more important, so it is kept; c is in another scope, so it stays.
        assert_eq!(
            duplicates(&facts),
            vec![("b".to_string(), vec!["a".to_string()])]
        );
        // A fact and its correction say the opposite: they are never merged.
        let corrected = [
            fact("x", "The user is allergic to peanuts", 2, None),
            fact("y", "The user is not allergic to peanuts", 3, None),
        ];
        assert!(duplicates(&corrected).is_empty());
    }

    #[test]
    fn scores_favour_what_is_used_and_fade_when_idle() {
        let mut used = artifact("skill", "active", Some("c"), 0, false);
        used.loaded = 10;
        used.used = 8;
        let mut ignored = used.clone();
        ignored.used = 0;
        let mut stale = used.clone();
        stale.last_used = Some(Utc::now() - Duration::days(60));
        let now = Utc::now();
        assert!(score(&used, now) > score(&ignored, now));
        assert!((score(&stale, now) - score(&used, now) / 4.0).abs() < 0.01);
    }

    #[test]
    fn only_the_assistant_s_unused_and_uncritical_artifacts_go() {
        let all = [
            artifact("skill", "active", Some("chat-1"), 40, false),
            artifact("skill", "active", Some("chat-1"), 40, true),
            artifact("skill", "active", None, 400, false),
            artifact("script_tool", "active", Some("chat-2"), 10, false),
            artifact("script_tool", "deprecated", Some("chat-3"), 200, false),
            artifact("script_tool", "candidate", Some("chat-4"), 200, false),
            artifact("tool", "active", Some("x"), 400, false),
        ];
        let d = decide(&all, &[], Utc::now());
        assert_eq!(
            d.deprecate,
            vec![("skill".into(), "skill-active-40".into())]
        );
        assert_eq!(
            d.archive,
            vec![
                ("script_tool".into(), "script_tool-deprecated-200".into()),
                ("script_tool".into(), "script_tool-candidate-200".into())
            ]
        );
        let failing = ToolStat {
            tool: "script__flaky".into(),
            calls: 6,
            failures: 4,
            avg_ms: 1,
        };
        let fine = ToolStat {
            tool: "script__ok".into(),
            calls: 6,
            failures: 1,
            avg_ms: 1,
        };
        let rare = ToolStat {
            tool: "script__rare".into(),
            calls: 2,
            failures: 2,
            avg_ms: 1,
        };
        assert_eq!(
            decide(&[], &[failing, fine, rare], Utc::now()).demote,
            vec!["flaky".to_string()]
        );
    }

    #[test]
    fn a_tool_that_got_worse_than_last_week_is_flagged() {
        let stat = |tool: &str, calls: i64, failures: i64| ToolStat {
            tool: tool.into(),
            calls,
            failures,
            avg_ms: 1,
        };
        let week = [
            stat("script__worse", 10, 4),
            stat("script__steady", 10, 4),
            stat("script__new", 10, 9),
            stat("web_search", 10, 9),
        ];
        let fortnight = [
            // Last week: 0 of 10 failed, then 4 of 10: worse by 0.4.
            stat("script__worse", 20, 4),
            // Last week: 4 of 10, the same as this week.
            stat("script__steady", 20, 8),
            // No calls last week: nothing to compare with.
            stat("script__new", 10, 9),
            stat("web_search", 20, 9),
        ];
        assert_eq!(regressed(&week, &fortnight), vec!["worse".to_string()]);
    }
}
