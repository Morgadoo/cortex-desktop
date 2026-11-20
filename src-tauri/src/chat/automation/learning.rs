//! Learning on its own, both off until turned on in Settings → Knowledge:
//!
//! - **Knowledge review** (LobeHub's memory agent): once a week, a background run reviews what
//!   is remembered with the `sanitize-my-knowledge` skill and leaves its proposals in an unread
//!   chat. Nothing changes until the user answers there.
//! - **Skills from work** (OpenBot): after an agent finishes a turn that took several tool
//!   steps, a short reflection asks whether it followed a procedure worth reusing. If so, the
//!   procedure is saved as a skill, turned off, and the user is told, to review and turn on.

use crate::support::settings_file::SettingsFile;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Learning {
    pub review_knowledge: bool,
    pub skills_from_work: bool,
    pub last_review: Option<DateTime<Utc>>,
    /// Run the user's eval suites once a day and say when a score drops. Off by default: every
    /// case costs model calls.
    pub nightly_evals: bool,
    pub last_evals: Option<DateTime<Utc>>,
}

const LEARNING_FILE: SettingsFile<Learning> = SettingsFile::sealed("chat/learning.json");

pub fn load(config_dir: &Path) -> Learning {
    LEARNING_FILE.load(config_dir)
}

pub fn save(config_dir: &Path, learning: &Learning) -> Result<(), String> {
    LEARNING_FILE
        .save(config_dir, learning)
        .map_err(String::from)
}

/// Whether the daily eval run is due (turned on, and a day since the last); marks it started.
pub(in crate::chat::automation) fn take_evals(config_dir: &Path, now: DateTime<Utc>) -> bool {
    let mut learning = load(config_dir);
    if !learning.nightly_evals
        || learning
            .last_evals
            .is_some_and(|t| now - t < Duration::days(1))
    {
        return false;
    }
    learning.last_evals = Some(now);
    save(config_dir, &learning).is_ok()
}

/// Whether the weekly review is due; marks it started when it is.
pub(in crate::chat::automation) fn take_review(config_dir: &Path, now: DateTime<Utc>) -> bool {
    let mut l = load(config_dir);
    let due = l.review_knowledge && l.last_review.is_none_or(|t| now - t >= Duration::days(7));
    if due {
        l.last_review = Some(now);
        let _ = save(config_dir, &l);
    }
    due
}

/// The task the review runs as: not stored, so it never shows on the Scheduled page. It also
/// diagnoses: the week's failures from the traces are in its prompt, so it reads what went wrong
/// and proposes fixes alongside the knowledge clean-up (the kernel's Diagnose step).
pub(in crate::chat::automation) fn review_task(config_dir: &Path) -> crate::chat::tasks::Task {
    let failures = crate::chat::traces::failure_report(config_dir, 7);
    crate::chat::tasks::Task {
        id: "knowledge-review".into(),
        title: "Weekly knowledge review".into(),
        prompt: format!(
            "Use the sanitize-my-knowledge skill to review everything remembered about me: the saved facts and every \
             knowledge page (list them with knowledge_search and open them with knowledge_read).\n\n\
             Then diagnose the past week from its traces, summarized below. For each tool or model that failed more \
             than once, open a failed run with query_traces and get_run, say what went wrong, and what would fix it \
             (a skill, a tool's description or script, a setting).\n\n<failure_summary>\n{failures}\n</failure_summary>\n\n\
             Do not change anything now: end with one table of proposed actions, knowledge and fixes, and ask me which \
             to apply. If everything is in order, say so in one line."
        ),
        frequency: crate::chat::tasks::Frequency::Once { date: String::new(), time: String::new() },
        agent_id: None,
        model: None,
        enabled: true,
        allow_changes: false,
        created_at: Utc::now(),
        last_run: None,
        last_conversation: None,
        last_status: None,
        next_run: None,
        seen: None,
        reply_in: None,
        webhook: None,
        comments: Vec::new(),
    }
}

/// Tool calls a turn needs before it is worth reflecting on.
pub const MIN_STEPS: usize = 3;

/// The reflection on a finished agent turn.
pub fn reflect_prompt(
    agent: &str,
    request: &str,
    steps: &[String],
    answer: &str,
    existing: &[String],
) -> String {
    format!(
        "The agent \"{agent}\" just did this for the user.\n\nRequest:\n{}\n\nSteps it took:\n{}\n\nResult (start):\n{}\n\n\
         Did it follow a procedure that would help next time a similar request comes, one that is not obvious and not \
         specific to this one case? If not, reply with exactly NONE. If so, reply with only a JSON object, no code fence: \
         {{\"name\": lowercase-hyphenated, 2 to 4 words, not one of {existing:?}; \"description\": \"Use when …\" in one \
         sentence; \"body\": the numbered steps, general enough to reuse, at most 150 words}}.",
        crate::support::text::clip(request, 1_500),
        steps.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"),
        crate::support::text::clip(answer, 1_500)
    )
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LearnedSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// The skill a reflection proposes, if any and if it is usable.
pub fn parse_reflection(text: &str, existing: &[String]) -> Option<LearnedSkill> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") || text.starts_with("NONE") {
        return None;
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let skill: LearnedSkill = serde_json::from_str(&text[start..=end]).ok()?;
    let name = skill.name.trim().to_lowercase();
    if !crate::chat::skills::valid_name(&name)
        || existing.contains(&name)
        || skill.body.trim().len() < 40
        || skill.description.trim().is_empty()
    {
        return None;
    }
    Some(LearnedSkill {
        name,
        description: skill.description.trim().to_string(),
        body: skill.body.trim().to_string(),
    })
}

/// Saves a learned skill, turned off, marked with the agent it came from.
pub fn keep(config_dir: &Path, agent: &str, skill: &LearnedSkill) -> Result<(), String> {
    let body = format!(
        "{}\n\n(Learned by {agent} from its own work; review before relying on it.)",
        skill.body
    );
    crate::chat::skills::save(config_dir, &skill.name, &skill.description, &body)?;
    // A candidate until the user turns it on (and, with evals, until it measures up).
    crate::chat::traces::register(
        config_dir,
        "skill",
        &skill.name,
        Some(&crate::chat::traces::hash(&body)),
        Some(agent),
        "candidate",
    );
    crate::chat::skills::set_enabled(config_dir, &skill.name, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviews_come_weekly_and_only_when_on() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        assert!(!take_review(dir.path(), now), "off by default");
        save(
            dir.path(),
            &Learning {
                review_knowledge: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(take_review(dir.path(), now));
        assert!(!take_review(dir.path(), now + Duration::days(3)));
        assert!(take_review(dir.path(), now + Duration::days(7)));
        let dir = tempfile::tempdir().unwrap();
        crate::chat::traces::start_run(dir.path(), "r1", "c1", None, "m");
        crate::chat::traces::tool_call(dir.path(), "r1", "script__flaky", 5, false);
        let task = review_task(dir.path());
        assert_eq!(task.title, "Weekly knowledge review");
        // Diagnose: the week's failures are in front of the review.
        assert!(task.prompt.contains("script__flaky"), "{}", task.prompt);
    }

    #[test]
    fn reflections_become_turned_off_skills() {
        let existing = vec!["weekly-report".to_string()];
        assert!(parse_reflection("NONE", &existing).is_none());
        assert!(parse_reflection("none.", &existing).is_none());
        let good = r#"Here: {"name": "Vendor-Price-Check", "description": "Use when comparing vendor prices.", "body": "1. List the vendors. 2. Search each one's price page. 3. Normalize to price per unit. 4. Table them."}"#;
        let skill = parse_reflection(good, &existing).unwrap();
        assert_eq!(skill.name, "vendor-price-check");
        assert!(
            parse_reflection(
                &good.replace("Vendor-Price-Check", "weekly-report"),
                &existing
            )
            .is_none(),
            "an existing name is not reused"
        );
        assert!(parse_reflection(r#"{"name": "x y", "description": "d", "body": "long enough body text that passes the length check easily"}"#, &existing).is_none());
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), "Researcher", &skill).unwrap();
        let saved = crate::chat::skills::find(dir.path(), "vendor-price-check").unwrap();
        assert!(!saved.enabled && saved.body.contains("Learned by Researcher"));
        assert!(reflect_prompt(
            "Researcher",
            "Compare",
            &["web_search: x".into()],
            "Done",
            &existing
        )
        .contains("\"weekly-report\""));
    }
}

#[cfg(test)]
mod nightly_tests {
    use super::*;

    #[test]
    fn nightly_evals_are_off_until_turned_on_and_then_daily() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        assert!(!take_evals(dir.path(), now));
        save(
            dir.path(),
            &Learning {
                nightly_evals: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(take_evals(dir.path(), now));
        assert!(!take_evals(dir.path(), now + Duration::hours(3)));
        assert!(take_evals(dir.path(), now + Duration::hours(25)));
    }
}
