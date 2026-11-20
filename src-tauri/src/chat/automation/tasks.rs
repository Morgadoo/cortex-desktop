//! Scheduled tasks: a prompt the app runs on its own at set times.
//!
//! Each run starts a new conversation (marked unread, under the task's agent when it has one)
//! with the prompt as its first message, and posts a notification when it finishes. Runs are
//! unattended: questions get "use your best judgement", and anything that would ask for approval
//! is refused unless the task allows changes. Times are the user's local time. The scheduler
//! wakes every 30 seconds while the app runs; a run missed while the app was closed happens once
//! at the next start.
//!
//! A task can also start on an event: new files in a folder (`NewFiles`). The folder's contents
//! are noted when the task is saved; each scheduler tick, files that appeared since and have not
//! changed for a few seconds start a run, with their names added to the prompt.

use crate::chat::error::ChatError;
use crate::support::settings_file::SettingsFile;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::chat::store::valid_id;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub enum Frequency {
    /// One run at a local date and time.
    #[serde(rename_all = "camelCase")]
    Once { date: String, time: String },
    #[serde(rename_all = "camelCase")]
    Daily { time: String },
    /// `days`: 0 = Monday … 6 = Sunday.
    #[serde(rename_all = "camelCase")]
    Weekly { days: Vec<u8>, time: String },
    /// `day` of the month, clamped to the month's length.
    #[serde(rename_all = "camelCase")]
    Monthly { day: u8, time: String },
    /// Every year on `month` (1–12) and `day`, clamped to the month's length.
    #[serde(rename_all = "camelCase")]
    Yearly { month: u8, day: u8, time: String },
    /// Whenever new files appear in `folder`.
    #[serde(rename_all = "camelCase")]
    NewFiles { folder: String },
    /// Whenever an RSS or Atom feed at `url` has new items (checked every 10 minutes).
    #[serde(rename_all = "camelCase")]
    NewFeedItems { url: String },
    /// A five-field cron expression (minute hour day-of-month month day-of-week), in local time:
    /// `0 9 * * 1-5` is 9:00 on weekdays.
    #[serde(rename_all = "camelCase")]
    Cron { expression: String },
}

/// The next time a cron expression fires after `after`, in the user's local time.
fn next_cron(expression: &str, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, ChatError> {
    let fields = expression.split_whitespace().count();
    if fields != 5 {
        return Err(ChatError::Invalid(format!(
            "`{expression}` is not a five-field cron expression like `0 9 * * 1-5`."
        )));
    }
    let cron: croner::Cron = expression
        .parse()
        .map_err(|e| ChatError::Invalid(format!("`{expression}` is not a cron expression: {e}")))?;
    Ok(cron
        .find_next_occurrence(&after.with_timezone(&Local), false)
        .ok()
        .map(|at| at.with_timezone(&Utc)))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Task {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub frequency: Frequency,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// `vendor/model`; the agent's, then the default model, when empty.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Let the run make changes (write files, run commands) without asking.
    #[serde(default)]
    pub allow_changes: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_conversation: Option<String>,
    /// `ok`, `failed` or `running`.
    #[serde(default)]
    pub last_status: Option<TaskStatus>,
    #[serde(default)]
    pub next_run: Option<DateTime<Utc>>,
    /// `NewFiles` tasks: the files already in the folder, so only newer ones start a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub seen: Option<Vec<String>>,
    /// Runs continue this conversation instead of starting a new one (a follow-up the user asked
    /// for in a chat: "check again tomorrow").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub reply_in: Option<String>,
    /// An address told when a run finishes (Open WebUI's webhooks): a JSON POST with the task,
    /// its status and the answer, for a script, a chat app or an automation service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub webhook: Option<String>,
    /// The user's comments on the task (LobeHub's task comments): shown on it, and given to every
    /// run as standing notes, so a comment steers the next run without rewriting the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<Vec<Comment>>", optional))]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Comment {
    pub at: DateTime<Utc>,
    pub text: String,
}

/// The prompt a run gets: the task's, with the user's comments as notes.
pub fn run_prompt(task: &Task) -> String {
    if task.comments.is_empty() {
        return task.prompt.clone();
    }
    let notes: Vec<String> = task
        .comments
        .iter()
        .map(|c| format!("- ({}) {}", c.at.format("%Y-%m-%d"), c.text))
        .collect();
    format!(
        "{}\n\nThe user's notes on this task, to follow (newest last):\n{}",
        task.prompt,
        notes.join("\n")
    )
}

/// Adds a comment to a task; `None` text removes the comment at `remove`.
pub fn comment(
    config_dir: &Path,
    id: &str,
    text: Option<&str>,
    remove: Option<usize>,
) -> Result<Vec<Task>, ChatError> {
    let mut tasks = load(config_dir);
    let task = tasks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| ChatError::Invalid("No such task.".into()))?;
    match (text.map(str::trim).filter(|t| !t.is_empty()), remove) {
        (Some(t), _) => task.comments.push(Comment {
            at: Utc::now(),
            text: crate::support::text::clip(t, 2_000),
        }),
        (None, Some(i)) if i < task.comments.len() => {
            task.comments.remove(i);
        }
        _ => return Err(ChatError::Invalid("Write a comment.".into())),
    }
    store(config_dir, &tasks)?;
    Ok(tasks)
}

/// How a task's latest run went, as saved in `tasks.json` and shown on the Tasks page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum TaskStatus {
    Running,
    Ok,
    Failed,
    /// A value this version does not know, from an older or newer one: shown as no status.
    #[serde(other)]
    Unknown,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

fn yes() -> bool {
    true
}

pub fn weekday_index(name: &str) -> Option<u8> {
    let w: Weekday = name.trim().parse().ok()?;
    Some(w.num_days_from_monday() as u8)
}

const TASKS_FILE: SettingsFile<Vec<Task>> = SettingsFile::sealed("chat/tasks.json");

pub fn load(config_dir: &Path) -> Vec<Task> {
    TASKS_FILE.load(config_dir)
}

fn store(config_dir: &Path, tasks: &[Task]) -> Result<(), ChatError> {
    TASKS_FILE
        .save(config_dir, tasks)
        .map_err(String::from)
        .map_err(ChatError::Invalid)
}

/// A planned run, for the calendar.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub task_id: String,
    pub at: DateTime<Utc>,
}

/// The planned runs of enabled tasks between `from` and `to`, in time order. Tasks started by
/// files have none.
pub fn occurrences(tasks: &[Task], from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for task in tasks.iter().filter(|t| t.enabled) {
        // Runs still to come: from now on, never before.
        let mut after = from.max(Utc::now()) - chrono::Duration::seconds(1);
        for _ in 0..400 {
            match next_run(&task.frequency, after) {
                Ok(Some(at)) if at <= to => {
                    out.push(Occurrence {
                        task_id: task.id.clone(),
                        at,
                    });
                    after = at;
                }
                _ => break,
            }
        }
    }
    out.sort_by_key(|o| o.at);
    out
}

/// Saves a task, validating it and computing its next run.
pub fn save(config_dir: &Path, mut task: Task) -> Result<Vec<Task>, ChatError> {
    if !valid_id(&task.id) {
        return Err(ChatError::Invalid("invalid task id".into()));
    }
    if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
        return Err(ChatError::Invalid(
            "A task needs a title and a prompt.".into(),
        ));
    }
    task.next_run = if task.enabled {
        next_run(&task.frequency, Utc::now())?
    } else {
        None
    };
    if let Frequency::NewFeedItems { url } = &task.frequency {
        if !url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return Err(ChatError::Invalid(
                "Give the feed's web address (https://…).".into(),
            ));
        }
        let previous = load(config_dir).into_iter().find(|t| t.id == task.id);
        task.seen = previous
            .filter(|p| p.frequency == task.frequency)
            .and_then(|p| p.seen);
    } else if let Frequency::NewFiles { folder } = &task.frequency {
        if !Path::new(folder).is_dir() {
            return Err(ChatError::Invalid("Choose a folder that exists.".into()));
        }
        // The same folder keeps what was already seen; a changed one starts a fresh baseline.
        let previous = load(config_dir).into_iter().find(|t| t.id == task.id);
        task.seen = previous
            .filter(|p| p.frequency == task.frequency)
            .and_then(|p| p.seen);
    } else if task.enabled && task.next_run.is_none() && task.last_run.is_none() {
        return Err(ChatError::Invalid("That time has already passed.".into()));
    }
    let mut tasks = load(config_dir);
    match tasks.iter_mut().find(|t| t.id == task.id) {
        Some(existing) => {
            // The task as it was stays restorable, like the rest of what can act on its own.
            crate::chat::versions::keep(config_dir, "tasks", &task.id, &*existing, &task)?;
            *existing = task;
        }
        None => tasks.push(task),
    }
    store(config_dir, &tasks)?;
    Ok(tasks)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Task>, ChatError> {
    let mut tasks = load(config_dir);
    tasks.retain(|t| t.id != id);
    store(config_dir, &tasks)?;
    Ok(tasks)
}

/// Marks the due tasks as started and returns them; their next runs are computed from now.
pub(in crate::chat::automation) fn take_due(config_dir: &Path, now: DateTime<Utc>) -> Vec<Task> {
    let mut tasks = load(config_dir);
    let mut due = Vec::new();
    for task in tasks.iter_mut() {
        if !task.enabled || task.next_run.is_none_or(|n| n > now) {
            continue;
        }
        task.last_run = Some(now);
        task.last_status = Some(TaskStatus::Running);
        task.next_run = next_run(&task.frequency, now).ok().flatten();
        if task.next_run.is_none() {
            // A one-off task is done once it ran.
            task.enabled = false;
        }
        due.push(task.clone());
    }
    if !due.is_empty() {
        if let Err(e) = store(config_dir, &tasks) {
            warn!("could not save task state: {e}");
        }
    }
    due
}

pub fn record(config_dir: &Path, id: &str, conversation_id: &str, status: TaskStatus) {
    let mut tasks = load(config_dir);
    if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
        task.last_conversation = Some(conversation_id.to_string());
        task.last_status = Some(status);
        let _ = store(config_dir, &tasks);
    }
}

mod feeds;
mod folders;
mod scheduler;
mod timing;
mod webhooks;
pub use feeds::*;
pub(in crate::chat::automation) use folders::*;
pub use scheduler::*;
pub use timing::*;
pub use webhooks::*;

#[cfg(test)]
mod tests {

    /// The scheduler's maintenance: each job runs when due and not again before its time, and
    /// what needs the app (the evals, the review turn) comes back for the loop to start.
    #[tokio::test]
    async fn maintenance_runs_each_job_when_it_is_due() {
        use crate::chat::{learning, providers, script_tools};
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let now = Utc::now();
        // A tool whose check now fails: the daily pass sends it back to candidate.
        script_tools::create(
            d,
            "",
            &serde_json::json!({ "name": "count_words", "description": "Count words.", "script": "function main(a) { return 2; }", "test_args": {} }),
        )
        .await
        .unwrap();
        script_tools::promote(d, "count_words", None).await.unwrap();
        let mut tool = script_tools::load(d, "count_words").unwrap();
        tool.checks[0].expected = "3".into();
        script_tools::save(d, &tool).unwrap();
        learning::save(
            d,
            &learning::Learning {
                review_knowledge: true,
                nightly_evals: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut models = providers::load(d);
        models.default_model = Some("anthropic/claude-haiku-5-5".into());
        providers::save(d, &models).unwrap();

        let due = maintenance(d, now).await;
        assert!(due.lifecycle.is_some());
        assert_eq!(
            due.checked.unwrap().demoted,
            vec!["count_words".to_string()]
        );
        assert_eq!(due.evals_on.as_deref(), Some("anthropic/claude-haiku-5-5"));
        assert_eq!(due.review.unwrap().title, "Weekly knowledge review");

        // An hour later nothing is due; a day later the daily jobs and evals, not the review.
        let later = maintenance(d, now + chrono::Duration::hours(1)).await;
        assert!(later.lifecycle.is_none() && later.checked.is_none());
        assert!(later.evals_on.is_none() && later.review.is_none());
        let tomorrow = maintenance(d, now + chrono::Duration::hours(25)).await;
        assert!(tomorrow.lifecycle.is_some() && tomorrow.evals_on.is_some());
        assert!(tomorrow.review.is_none());
    }

    /// `schedule(cron, task)`: a cron expression, in local time, through the same next-run path.
    #[test]
    fn saved_statuses_read_back_whatever_wrote_them() {
        for (text, status) in [
            ("\"running\"", TaskStatus::Running),
            ("\"ok\"", TaskStatus::Ok),
            ("\"failed\"", TaskStatus::Failed),
            ("\"paused\"", TaskStatus::Unknown),
        ] {
            assert_eq!(serde_json::from_str::<TaskStatus>(text).unwrap(), status);
        }
        assert_eq!(serde_json::to_string(&TaskStatus::Ok).unwrap(), "\"ok\"");
    }

    #[test]
    fn cron_tasks_run_on_their_expression() {
        use chrono::Timelike;
        let cron = |e: &str| Frequency::Cron {
            expression: e.into(),
        };
        let after = Local
            .with_ymd_and_hms(2030, 1, 7, 8, 0, 0) // a Monday
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let next = next_run(&cron("30 9 * * 1-5"), after).unwrap().unwrap();
        let local = next.with_timezone(&Local);
        assert_eq!((local.day(), local.hour(), local.minute()), (7, 9, 30));
        let saturday = Local
            .with_ymd_and_hms(2030, 1, 12, 8, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let monday = next_run(&cron("30 9 * * 1-5"), saturday)
            .unwrap()
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(monday.day(), 14, "skips the weekend");
        assert!(next_run(&cron("every day"), after).is_err());
        assert!(next_run(&cron("61 9 * * *"), after).is_err());
        assert_eq!(describe(&cron("0 9 * * *")), "On the schedule `0 9 * * *`");
    }

    /// The loop's part: what is due is started, and nothing else.
    #[test]
    fn the_scheduler_starts_what_is_due() {
        let started = std::cell::RefCell::new(Vec::new());
        let due = Maintenance {
            evals_on: Some("m/a".into()),
            review: Some(crate::chat::learning::review_task(std::path::Path::new(
                "/nonexistent",
            ))),
            ..Maintenance::default()
        };
        dispatch(
            due,
            |model| started.borrow_mut().push(format!("evals on {model}")),
            |task| started.borrow_mut().push(task.title),
        );
        assert_eq!(
            started.into_inner(),
            vec![
                "evals on m/a".to_string(),
                "Weekly knowledge review".to_string()
            ]
        );
        let nothing = std::cell::Cell::new(0);
        dispatch(
            Maintenance::default(),
            |_| nothing.set(nothing.get() + 1),
            |_| nothing.set(nothing.get() + 1),
        );
        assert_eq!(nothing.get(), 0);
    }

    #[test]
    fn a_nightly_drop_is_announced() {
        use crate::chat::evals::{drop_notice, Nightly};
        let result = |score: f64, before: Option<f64>| Nightly {
            suite: "trips".into(),
            score,
            before,
            worse: vec!["porto".into()],
        };
        let (title, body) = drop_notice(&result(0.5, Some(0.75))).unwrap();
        assert_eq!(title, "An eval score dropped");
        assert_eq!(body, "trips: 0.50 (was 0.75). Worse: porto");
        assert!(drop_notice(&result(0.75, Some(0.75))).is_none());
        assert!(drop_notice(&result(0.1, None)).is_none());
    }

    /// A task is versioned like the rest of what acts on its own: an edit can be undone.
    #[test]
    fn a_task_edit_can_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut task = crate::chat::learning::review_task(d);
        task.id = "report".into();
        task.prompt = "Write the weekly report.".into();
        task.frequency = Frequency::Weekly {
            days: vec![0],
            time: "09:00".into(),
        };
        save(d, task.clone()).unwrap();
        let mut changed = task.clone();
        changed.prompt = "Email everyone the report.".into();
        save(d, changed).unwrap();
        let before = crate::chat::versions::list(d, "tasks", "report")
            .unwrap()
            .remove(0);
        crate::chat::history::restore(d, "tasks", "report", &before.id).unwrap();
        assert_eq!(load(d)[0].prompt, "Write the weekly report.");
    }

    #[test]
    fn the_calendar_lists_planned_runs_in_range() {
        let base = Task {
            id: "d".into(),
            title: "Daily".into(),
            prompt: "x".into(),
            frequency: Frequency::Daily {
                time: "09:00".into(),
            },
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
        };
        let mut paused = base.clone();
        paused.id = "p".into();
        paused.enabled = false;
        let mut folder = base.clone();
        folder.id = "f".into();
        folder.frequency = Frequency::NewFiles {
            folder: "/tmp".into(),
        };
        let from = Utc::now();
        let runs = occurrences(
            &[base, paused, folder],
            from,
            from + chrono::Duration::days(7),
        );
        assert!(runs.len() >= 7 && runs.len() <= 8, "{}", runs.len());
        assert!(runs.iter().all(|o| o.task_id == "d" && o.at > from));
        assert!(runs.windows(2).all(|w| w[0].at < w[1].at));
    }

    #[test]
    fn new_files_start_a_run_once_settled() {
        let config = tempfile::tempdir().unwrap();
        let watched = tempfile::tempdir().unwrap();
        crate::chat::at_rest::write(watched.path().join("old.pdf"), "x").unwrap();
        let task = Task {
            id: "t1".into(),
            title: "File invoices".into(),
            prompt: "Summarize each new invoice.".into(),
            frequency: Frequency::NewFiles {
                folder: watched.path().to_string_lossy().into_owned(),
            },
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
        };
        save(config.path(), task).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(60);
        assert!(
            take_new_files(config.path(), later).is_empty(),
            "the first look only notes what is there"
        );
        crate::chat::at_rest::write(watched.path().join("march.pdf"), "x").unwrap();
        crate::chat::at_rest::write(watched.path().join(".hidden"), "x").unwrap();
        // Just written: not settled yet.
        assert!(take_new_files(config.path(), std::time::SystemTime::now()).is_empty());
        let runs = take_new_files(config.path(), later);
        assert_eq!(runs.len(), 1);
        assert!(
            runs[0].prompt.contains("march.pdf")
                && !runs[0].prompt.contains("old.pdf")
                && !runs[0].prompt.contains(".hidden")
        );
        assert!(
            take_new_files(config.path(), later).is_empty(),
            "each file starts one run"
        );
        assert_eq!(
            describe(&load(config.path())[0].frequency),
            format!(
                "When new files arrive in {}",
                watched.path().file_name().unwrap().to_string_lossy()
            )
        );
        let mut gone = load(config.path())[0].clone();
        gone.frequency = Frequency::NewFiles {
            folder: "/no/such/folder".into(),
        };
        assert!(save(config.path(), gone).is_err());
    }

    use super::*;

    #[test]
    fn webhooks_get_the_answer_in_shapes_chat_apps_read() {
        let task = Task {
            id: "t1".into(),
            title: "Morning brief".into(),
            prompt: "x".into(),
            frequency: Frequency::Daily {
                time: "08:00".into(),
            },
            agent_id: None,
            model: None,
            enabled: true,
            allow_changes: false,
            created_at: chrono::Utc::now(),
            last_run: None,
            last_conversation: None,
            last_status: None,
            next_run: None,
            seen: None,
            reply_in: None,
            webhook: Some("https://example.com/hook".into()),
            comments: Vec::new(),
        };
        let p = webhook_payload(&task, "c1", TaskStatus::Ok, "Three items today.");
        assert_eq!(
            (
                p["event"].as_str(),
                p["status"].as_str(),
                p["conversationId"].as_str()
            ),
            (Some("task.finished"), Some("ok"), Some("c1"))
        );
        assert_eq!(p["text"], "Morning brief: Three items today.");
        assert!(p["content"]
            .as_str()
            .unwrap()
            .starts_with("**Morning brief**"));
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn yearly_runs_on_its_date_and_clamps_leap_days() {
        let f = Frequency::Yearly {
            month: 3,
            day: 15,
            time: "10:00".into(),
        };
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 8, 0)).unwrap(),
            Some(at(2027, 3, 15, 10, 0))
        );
        assert_eq!(
            next_run(&f, at(2027, 3, 14, 8, 0)).unwrap(),
            Some(at(2027, 3, 15, 10, 0))
        );
        let leap = Frequency::Yearly {
            month: 2,
            day: 29,
            time: "09:00".into(),
        };
        assert_eq!(
            next_run(&leap, at(2026, 11, 17, 8, 0)).unwrap(),
            Some(at(2027, 2, 28, 9, 0))
        );
        assert!(next_run(
            &Frequency::Yearly {
                month: 13,
                day: 1,
                time: "09:00".into()
            },
            at(2026, 1, 1, 0, 0)
        )
        .is_err());
        assert_eq!(describe(&f), "Every year on March 15 at 10:00");
    }

    #[test]
    fn daily_runs_today_or_tomorrow() {
        let f = Frequency::Daily {
            time: "09:30".into(),
        };
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 8, 0)).unwrap(),
            Some(at(2026, 11, 17, 9, 30))
        );
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 9, 30)).unwrap(),
            Some(at(2026, 11, 18, 9, 30))
        );
    }

    #[test]
    fn weekly_finds_the_next_listed_day() {
        // 2026-11-17 is a Tuesday.
        let f = Frequency::Weekly {
            days: vec![0, 4],
            time: "10:00".into(),
        };
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 12, 0)).unwrap(),
            Some(at(2026, 11, 20, 10, 0))
        );
        assert_eq!(
            next_run(&f, at(2026, 11, 20, 11, 0)).unwrap(),
            Some(at(2026, 11, 23, 10, 0))
        );
        assert!(next_run(
            &Frequency::Weekly {
                days: vec![],
                time: "10:00".into()
            },
            Utc::now()
        )
        .is_err());
    }

    #[test]
    fn monthly_clamps_to_the_month() {
        let f = Frequency::Monthly {
            day: 31,
            time: "08:00".into(),
        };
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 0, 0)).unwrap(),
            Some(at(2026, 11, 30, 8, 0))
        );
    }

    #[test]
    fn once_runs_once() {
        let f = Frequency::Once {
            date: "2026-12-01".into(),
            time: "09:00".into(),
        };
        assert_eq!(
            next_run(&f, at(2026, 11, 17, 0, 0)).unwrap(),
            Some(at(2026, 12, 1, 9, 0))
        );
        assert_eq!(next_run(&f, at(2026, 12, 2, 0, 0)).unwrap(), None);
        assert!(next_run(&Frequency::Daily { time: "9am".into() }, Utc::now()).is_err());
    }

    #[test]
    fn descriptions_read_naturally() {
        assert_eq!(
            describe(&Frequency::Weekly {
                days: vec![0],
                time: "05:30".into()
            }),
            "Every Monday at 05:30"
        );
        assert_eq!(
            describe(&Frequency::Weekly {
                days: vec![4, 0, 2],
                time: "09:00".into()
            }),
            "Every Monday, Wednesday and Friday at 09:00"
        );
        assert_eq!(
            describe(&Frequency::Weekly {
                days: vec![0, 1, 2, 3, 4],
                time: "09:00".into()
            }),
            "Every weekday at 09:00"
        );
        assert_eq!(weekday_index("friday"), Some(4));
        assert_eq!(weekday_index("Mon"), Some(0));
    }

    #[test]
    fn due_tasks_are_taken_once_and_rescheduled() {
        let dir = tempfile::tempdir().unwrap();
        let task = Task {
            id: "t1".into(),
            title: "Brief".into(),
            prompt: "News".into(),
            frequency: Frequency::Daily {
                time: "09:00".into(),
            },
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
        };
        save(dir.path(), task).unwrap();
        let next = load(dir.path())[0].next_run.unwrap();
        assert!(take_due(dir.path(), next - chrono::Duration::minutes(1)).is_empty());
        assert_eq!(take_due(dir.path(), next).len(), 1);
        assert!(take_due(dir.path(), next).is_empty());
        let saved = &load(dir.path())[0];
        assert!(saved.next_run.unwrap() > next);
        assert_eq!(saved.last_status, Some(TaskStatus::Running));
    }

    #[test]
    fn comments_reach_the_run_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = load(dir.path());
        t.push(Task {
            id: "t1".into(),
            title: "Brief".into(),
            prompt: "Summarize the news.".into(),
            frequency: Frequency::Daily {
                time: "09:00".into(),
            },
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
        });
        store(dir.path(), &t).unwrap();
        assert_eq!(run_prompt(&load(dir.path())[0]), "Summarize the news.");
        comment(dir.path(), "t1", Some("Skip sports."), None).unwrap();
        comment(dir.path(), "t1", Some("  Keep it to five bullets. "), None).unwrap();
        let p = run_prompt(&load(dir.path())[0]);
        assert!(
            p.starts_with("Summarize the news.\n\nThe user's notes")
                && p.contains("Skip sports.")
                && p.ends_with("Keep it to five bullets."),
            "{p}"
        );
        comment(dir.path(), "t1", None, Some(0)).unwrap();
        assert_eq!(load(dir.path())[0].comments.len(), 1);
        assert!(comment(dir.path(), "t1", Some("  "), None).is_err());
    }

    #[test]
    fn rss_and_atom_feeds_are_read() {
        let rss = r#"<?xml version="1.0"?><rss><channel><title>Blog</title><link>https://b.example</link>
            <item><title><![CDATA[Rust & Tauri]]></title><link>https://b.example/2</link><guid>post-2</guid></item>
            <item><title>First</title><link>https://b.example/1</link></item></channel></rss>"#;
        let items = parse_feed(rss);
        assert_eq!(
            items,
            vec![
                FeedItem {
                    id: "post-2".into(),
                    title: "Rust & Tauri".into(),
                    link: "https://b.example/2".into()
                },
                FeedItem {
                    id: "https://b.example/1".into(),
                    title: "First".into(),
                    link: "https://b.example/1".into()
                },
            ]
        );
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><link href="https://a.example"/>
            <entry><title type="text">News &amp; notes</title><link rel="alternate" href="https://a.example/n?x=1&amp;y=2"/><id>tag:a,1</id></entry></feed>"#;
        assert_eq!(
            parse_feed(atom),
            vec![FeedItem {
                id: "tag:a,1".into(),
                title: "News & notes".into(),
                link: "https://a.example/n?x=1&y=2".into()
            }]
        );
        assert!(parse_feed("not a feed").is_empty());
        assert_eq!(
            describe(&Frequency::NewFeedItems {
                url: "https://blog.rust-lang.org/feed.xml".into()
            }),
            "When blog.rust-lang.org posts something new"
        );
        assert_eq!(
            next_run(
                &Frequency::NewFeedItems {
                    url: "https://x".into()
                },
                Utc::now()
            )
            .unwrap(),
            None
        );
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    /// Time-based tasks are timed by the same engine as the web app's schedules.
    #[test]
    fn tasks_and_schedules_share_one_timing_engine() {
        if iana_time_zone::get_timezone().is_err() {
            // In CI the runner has a time zone, so a skip there would hide a failure.
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs a time zone for this test"
            );
            return;
        }
        let after = Utc::now();
        for frequency in [
            Frequency::Daily {
                time: "09:30".into(),
            },
            Frequency::Weekly {
                days: vec![0, 4],
                time: "18:00".into(),
            },
            Frequency::Monthly {
                day: 15,
                time: "07:05".into(),
            },
        ] {
            assert!(calendar(&frequency).is_some(), "{frequency:?}");
            let next = next_run(&frequency, after).unwrap().unwrap();
            assert!(next > after);
            let local = next.with_timezone(&Local);
            match &frequency {
                Frequency::Daily { .. } => {
                    assert_eq!(local.format("%H:%M").to_string(), "09:30")
                }
                Frequency::Weekly { .. } => {
                    assert!(matches!(local.weekday(), Weekday::Mon | Weekday::Fri));
                    assert_eq!(local.format("%H:%M").to_string(), "18:00");
                }
                _ => assert_eq!(local.day(), 15),
            }
        }
        // The 31st runs on a short month's last day, so it keeps its own rule.
        assert!(calendar(&Frequency::Monthly {
            day: 31,
            time: "09:00".into()
        })
        .is_none());
    }
}
