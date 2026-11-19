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
                j != i && !taken[j] && facts[j].scope == facts[i].scope && {
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
            "deprecated" if idle > Duration::days(ARCHIVE_DAYS) => {
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
            artifact("tool", "active", Some("x"), 400, false),
        ];
        let d = decide(&all, &[], Utc::now());
        assert_eq!(
            d.deprecate,
            vec![("skill".into(), "skill-active-40".into())]
        );
        assert_eq!(
            d.archive,
            vec![("script_tool".into(), "script_tool-deprecated-200".into())]
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
}
