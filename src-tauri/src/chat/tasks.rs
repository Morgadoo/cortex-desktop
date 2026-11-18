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

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::store::{chat_dir, valid_id, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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
    pub last_status: Option<String>,
    #[serde(default)]
    pub next_run: Option<DateTime<Utc>>,
    /// `NewFiles` tasks: the files already in the folder, so only newer ones start a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Vec<String>>,
    /// Runs continue this conversation instead of starting a new one (a follow-up the user asked
    /// for in a chat: "check again tomorrow").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_in: Option<String>,
    /// An address told when a run finishes (Open WebUI's webhooks): a JSON POST with the task,
    /// its status and the answer, for a script, a chat app or an automation service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<String>,
    /// The user's comments on the task (LobeHub's task comments): shown on it, and given to every
    /// run as standing notes, so a comment steers the next run without rewriting the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub at: DateTime<Utc>,
    pub text: String,
}

/// The prompt a run gets: the task's, with the user's comments as notes.
pub fn run_prompt(task: &Task) -> String {
    if task.comments.is_empty() {
        return task.prompt.clone();
    }
    let notes: Vec<String> = task.comments.iter().map(|c| format!("- ({}) {}", c.at.format("%Y-%m-%d"), c.text)).collect();
    format!("{}\n\nThe user's notes on this task, to follow (newest last):\n{}", task.prompt, notes.join("\n"))
}

/// Adds a comment to a task; `None` text removes the comment at `remove`.
pub fn comment(config_dir: &Path, id: &str, text: Option<&str>, remove: Option<usize>) -> Result<Vec<Task>, String> {
    let mut tasks = load(config_dir);
    let task = tasks.iter_mut().find(|t| t.id == id).ok_or("No such task.")?;
    match (text.map(str::trim).filter(|t| !t.is_empty()), remove) {
        (Some(t), _) => task.comments.push(Comment { at: Utc::now(), text: t.chars().take(2_000).collect() }),
        (None, Some(i)) if i < task.comments.len() => {
            task.comments.remove(i);
        }
        _ => return Err("Write a comment.".into()),
    }
    store(config_dir, &tasks)?;
    Ok(tasks)
}

/// What a webhook receives when a run finishes.
pub fn webhook_payload(task: &Task, conversation_id: &str, status: &str, answer: &str) -> serde_json::Value {
    let answer: String = answer.chars().take(20_000).collect();
    serde_json::json!({
        "event": "task.finished",
        "task": { "id": task.id, "title": task.title },
        "status": status,
        "conversationId": conversation_id,
        "answer": answer,
        // Slack and Discord incoming webhooks read these fields.
        "text": format!("{}: {}", task.title, answer.chars().take(3_000).collect::<String>()),
        "content": format!("**{}**\n{}", task.title, answer.chars().take(1_900).collect::<String>()),
        "finishedAt": chrono::Utc::now(),
    })
}

/// Posts a finished run to the task's webhook, when it has one (http or https only).
pub async fn notify_webhook(task: &Task, conversation_id: &str, status: &str, answer: &str) {
    let Some(url) = task.webhook.as_deref().map(str::trim).filter(|u| u.starts_with("https://") || u.starts_with("http://")) else { return };
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build() else { return };
    match client.post(url).json(&webhook_payload(task, conversation_id, status, answer)).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => log::warn!("the webhook of {} answered {}", task.title, r.status()),
        Err(e) => log::warn!("the webhook of {} failed: {e}", task.title),
    }
}

fn yes() -> bool {
    true
}

fn parse_time(time: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(time.trim(), "%H:%M").map_err(|_| format!("`{time}` is not a time like 09:30"))
}

fn local(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    Local.from_local_datetime(&date.and_time(time)).earliest().map(|t| t.with_timezone(&Utc))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) };
    next.and_then(|n| n.pred_opt()).map_or(28, |d| d.day())
}

/// The first run strictly after `after`, or `None` when there is no other.
pub fn next_run(frequency: &Frequency, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, String> {
    let now = after.with_timezone(&Local);
    let today = now.date_naive();
    match frequency {
        Frequency::Once { date, time } => {
            let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|_| format!("`{date}` is not a date like 2026-12-01"))?;
            let at = local(date, parse_time(time)?);
            Ok(at.filter(|t| *t > after))
        }
        Frequency::Daily { time } => {
            let time = parse_time(time)?;
            Ok((0..3).filter_map(|d| local(today + chrono::Days::new(d), time)).find(|t| *t > after))
        }
        Frequency::Weekly { days, time } => {
            if days.is_empty() || days.iter().any(|d| *d > 6) {
                return Err("Weekly tasks need days from 0 (Monday) to 6 (Sunday).".into());
            }
            let time = parse_time(time)?;
            Ok((0..15)
                .map(|d| today + chrono::Days::new(d))
                .filter(|date| days.contains(&(date.weekday().num_days_from_monday() as u8)))
                .filter_map(|date| local(date, time))
                .find(|t| *t > after))
        }
        Frequency::Monthly { day, time } => {
            if *day == 0 || *day > 31 {
                return Err("A monthly task runs on a day from 1 to 31.".into());
            }
            let time = parse_time(time)?;
            let mut year = today.year();
            let mut month = today.month();
            for _ in 0..14 {
                let d = u32::from(*day).min(days_in_month(year, month));
                if let Some(at) = NaiveDate::from_ymd_opt(year, month, d).and_then(|date| local(date, time)) {
                    if at > after {
                        return Ok(Some(at));
                    }
                }
                if month == 12 {
                    year += 1;
                    month = 1;
                } else {
                    month += 1;
                }
            }
            Ok(None)
        }
        // Started by files or feeds, not by the clock.
        Frequency::NewFiles { .. } | Frequency::NewFeedItems { .. } => Ok(None),
        Frequency::Yearly { month, day, time } => {
            if *month == 0 || *month > 12 || *day == 0 || *day > 31 {
                return Err("A yearly task needs a month from 1 to 12 and a day from 1 to 31.".into());
            }
            let time = parse_time(time)?;
            for year in today.year()..=today.year() + 2 {
                let d = u32::from(*day).min(days_in_month(year, u32::from(*month)));
                if let Some(at) = NaiveDate::from_ymd_opt(year, u32::from(*month), d).and_then(|date| local(date, time)) {
                    if at > after {
                        return Ok(Some(at));
                    }
                }
            }
            Ok(None)
        }
    }
}

/// "Every Monday and Thursday at 09:00", in the words the task list shows.
pub fn describe(frequency: &Frequency) -> String {
    const NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    match frequency {
        Frequency::Once { date, time } => format!("Once on {date} at {time}"),
        Frequency::Daily { time } => format!("Every day at {time}"),
        Frequency::Weekly { days, time } => {
            let mut days = days.clone();
            days.sort_unstable();
            days.dedup();
            let names: Vec<&str> = days.iter().filter_map(|d| NAMES.get(*d as usize).copied()).collect();
            let list = match names.as_slice() {
                [one] => (*one).to_string(),
                [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                [] => String::new(),
            };
            if days == [0, 1, 2, 3, 4] {
                format!("Every weekday at {time}")
            } else {
                format!("Every {list} at {time}")
            }
        }
        Frequency::Monthly { day, time } => format!("Every month on day {day} at {time}"),
        Frequency::Yearly { month, day, time } => {
            const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
            let name = MONTHS.get(usize::from(month.saturating_sub(1))).copied().unwrap_or("?");
            format!("Every year on {name} {day} at {time}")
        }
        Frequency::NewFiles { folder } => {
            let name = Path::new(folder).file_name().map_or_else(|| folder.clone(), |n| n.to_string_lossy().into_owned());
            format!("When new files arrive in {name}")
        }
        Frequency::NewFeedItems { url } => {
            let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.clone());
            format!("When {host} posts something new")
        }
    }
}

/// One item of a feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub id: String,
    pub title: String,
    pub link: String,
}

/// The text of the first `<tag>` in `xml` (CDATA and entities undone), if any.
fn element<'a>(xml: &'a str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let start = from + xml[from..].find(&open)?;
        let after = &xml[start + open.len()..];
        // `<link` must not match `<linkedin`: the tag name ends at a space, `>` or `/`.
        if !after.starts_with(['>', ' ', '/', '\t', '\n', '\r']) {
            from = start + open.len();
            continue;
        }
        let gt = start + xml[start..].find('>')?;
        if xml[..gt].ends_with('/') {
            return Some(String::new());
        }
        let close = format!("</{tag}>");
        let end = gt + 1 + xml[gt + 1..].find(&close)?;
        let raw = xml[gt + 1..end].trim();
        let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(raw);
        return Some(raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&").trim().to_string());
    }
}

/// An attribute of the first `<tag …>` in `xml`.
fn attribute(xml: &str, tag: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("<{tag} "))?;
    let end = start + xml[start..].find('>')?;
    let head = &xml[start..end];
    for q in ['"', '\''] {
        if let Some(i) = head.find(&format!("{name}={q}")) {
            let rest = &head[i + name.len() + 2..];
            return rest.find(q).map(|j| rest[..j].replace("&amp;", "&"));
        }
    }
    None
}

/// The items of an RSS 2.0, RSS 1.0 or Atom feed, newest first as published.
pub fn parse_feed(xml: &str) -> Vec<FeedItem> {
    let (open, close) = if xml.contains("<entry") { ("<entry", "</entry>") } else { ("<item", "</item>") };
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let Some(len) = rest[start..].find(close) else { break };
        let block = &rest[start..start + len];
        rest = &rest[start + len + close.len()..];
        let title = element(block, "title").unwrap_or_default();
        let link = element(block, "link").filter(|l| !l.is_empty()).or_else(|| attribute(block, "link", "href")).unwrap_or_default();
        let id = element(block, "guid").or_else(|| element(block, "id")).filter(|i| !i.is_empty()).unwrap_or_else(|| if link.is_empty() { title.clone() } else { link.clone() });
        if !id.is_empty() {
            items.push(FeedItem { id, title, link });
        }
    }
    items
}

/// Feed tasks whose feeds have new items: each with the prompt naming them. Feeds are fetched
/// at most every 10 minutes per task.
pub async fn take_new_feed_items(config_dir: &Path) -> Vec<Task> {
    let due: Vec<(String, String)> = load(config_dir)
        .into_iter()
        .filter(|t| t.enabled)
        .filter_map(|t| match &t.frequency {
            Frequency::NewFeedItems { url } => Some((t.id.clone(), url.clone())),
            _ => None,
        })
        .collect();
    if due.is_empty() {
        return Vec::new();
    }
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).user_agent("Lumo feed reader").build();
    let Ok(client) = client else { return Vec::new() };
    let mut fetched = Vec::new();
    for (id, url) in due {
        match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => match r.text().await {
                Ok(text) => fetched.push((id, parse_feed(&text))),
                Err(e) => warn!("feed {url}: {e}"),
            },
            Err(e) => warn!("feed {url}: {e}"),
        }
    }
    let mut tasks = load(config_dir);
    let mut started = Vec::new();
    for (id, items) in fetched {
        let Some(task) = tasks.iter_mut().find(|t| t.id == id) else { continue };
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let Some(seen) = task.seen.as_mut() else {
            // First look: what the feed shows now is the baseline.
            task.seen = Some(ids);
            continue;
        };
        let fresh: Vec<&FeedItem> = items.iter().filter(|i| !seen.contains(&i.id)).take(20).collect();
        // Remember what the feed shows now (feeds drop old items), plus anything still listed.
        let mut keep = ids.clone();
        keep.extend(seen.iter().filter(|s| !ids.contains(s)).take(200).cloned());
        *seen = keep;
        if fresh.is_empty() {
            continue;
        }
        task.last_run = Some(Utc::now());
        task.last_status = Some("running".into());
        let mut run = task.clone();
        let list: Vec<String> = fresh.iter().map(|i| if i.link.is_empty() { format!("- {}", i.title) } else { format!("- {} ({})", i.title, i.link) }).collect();
        let url = match &task.frequency {
            Frequency::NewFeedItems { url } => url.clone(),
            _ => String::new(),
        };
        run.prompt = format!("{}\n\nNew in the feed {url}:\n{}", task.prompt, list.join("\n"));
        started.push(run);
    }
    if let Err(e) = store(config_dir, &tasks) {
        warn!("could not save task state: {e}");
    }
    started
}

/// How long a new file must stay unchanged before it counts (it may still be copying).
const SETTLE: Duration = Duration::from_secs(5);

/// The visible files directly in `folder`, with whether each has settled.
fn folder_files(folder: &Path, now: std::time::SystemTime) -> Result<Vec<(String, bool)>, String> {
    let entries = std::fs::read_dir(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else { continue };
        // Hidden files and partial downloads are not new documents.
        if !meta.is_file() || name.starts_with('.') || name.ends_with(".part") || name.ends_with(".crdownload") || name.ends_with(".tmp") {
            continue;
        }
        let settled = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_none_or(|age| age >= SETTLE);
        out.push((name, settled));
    }
    out.sort();
    Ok(out)
}

/// The folder tasks whose folders have new, settled files: each with the prompt naming them.
pub fn take_new_files(config_dir: &Path, now: std::time::SystemTime) -> Vec<Task> {
    let mut tasks = load(config_dir);
    let mut started = Vec::new();
    let mut changed = false;
    for task in tasks.iter_mut().filter(|t| t.enabled) {
        let Frequency::NewFiles { folder } = &task.frequency else { continue };
        let Ok(files) = folder_files(Path::new(folder), now) else { continue };
        let Some(seen) = task.seen.as_mut() else {
            // First look: what is there now is the baseline.
            task.seen = Some(files.into_iter().map(|(n, _)| n).collect());
            changed = true;
            continue;
        };
        let fresh: Vec<String> = files.iter().filter(|(n, settled)| *settled && !seen.contains(n)).map(|(n, _)| n.clone()).collect();
        // Files that went away may come back as new ones later.
        let before = seen.len();
        seen.retain(|n| files.iter().any(|(f, _)| f == n));
        changed |= seen.len() != before;
        if fresh.is_empty() {
            continue;
        }
        seen.extend(fresh.iter().cloned());
        changed = true;
        task.last_run = Some(Utc::now());
        task.last_status = Some("running".into());
        let mut run = task.clone();
        let list: Vec<String> = fresh.iter().map(|n| format!("- {}", Path::new(folder).join(n).display())).collect();
        run.prompt = format!("{}\n\nNew files in `{folder}`:\n{}", task.prompt, list.join("\n"));
        started.push(run);
    }
    if changed {
        if let Err(e) = store(config_dir, &tasks) {
            warn!("could not save task state: {e}");
        }
    }
    started
}

pub fn weekday_index(name: &str) -> Option<u8> {
    let w: Weekday = name.trim().parse().ok()?;
    Some(w.num_days_from_monday() as u8)
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("tasks.json")
}

pub fn load(config_dir: &Path) -> Vec<Task> {
    crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store(config_dir: &Path, tasks: &[Task]) -> Result<(), String> {
    write_json(&path(config_dir), &tasks)
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
                    out.push(Occurrence { task_id: task.id.clone(), at });
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
pub fn save(config_dir: &Path, mut task: Task) -> Result<Vec<Task>, String> {
    if !valid_id(&task.id) {
        return Err("invalid task id".into());
    }
    if task.title.trim().is_empty() || task.prompt.trim().is_empty() {
        return Err("A task needs a title and a prompt.".into());
    }
    task.next_run = if task.enabled { next_run(&task.frequency, Utc::now())? } else { None };
    if let Frequency::NewFeedItems { url } = &task.frequency {
        if !url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return Err("Give the feed's web address (https://…).".into());
        }
        let previous = load(config_dir).into_iter().find(|t| t.id == task.id);
        task.seen = previous.filter(|p| p.frequency == task.frequency).and_then(|p| p.seen);
    } else if let Frequency::NewFiles { folder } = &task.frequency {
        if !Path::new(folder).is_dir() {
            return Err("Choose a folder that exists.".into());
        }
        // The same folder keeps what was already seen; a changed one starts a fresh baseline.
        let previous = load(config_dir).into_iter().find(|t| t.id == task.id);
        task.seen = previous.filter(|p| p.frequency == task.frequency).and_then(|p| p.seen);
    } else if task.enabled && task.next_run.is_none() && task.last_run.is_none() {
        return Err("That time has already passed.".into());
    }
    let mut tasks = load(config_dir);
    match tasks.iter_mut().find(|t| t.id == task.id) {
        Some(existing) => *existing = task,
        None => tasks.push(task),
    }
    store(config_dir, &tasks)?;
    Ok(tasks)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Task>, String> {
    let mut tasks = load(config_dir);
    tasks.retain(|t| t.id != id);
    store(config_dir, &tasks)?;
    Ok(tasks)
}

/// Marks the due tasks as started and returns them; their next runs are computed from now.
pub fn take_due(config_dir: &Path, now: DateTime<Utc>) -> Vec<Task> {
    let mut tasks = load(config_dir);
    let mut due = Vec::new();
    for task in tasks.iter_mut() {
        if !task.enabled || task.next_run.is_none_or(|n| n > now) {
            continue;
        }
        task.last_run = Some(now);
        task.last_status = Some("running".into());
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

pub fn record(config_dir: &Path, id: &str, conversation_id: &str, status: &str) {
    let mut tasks = load(config_dir);
    if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
        task.last_conversation = Some(conversation_id.to_string());
        task.last_status = Some(status.to_string());
        let _ = store(config_dir, &tasks);
    }
}

/// Starts the scheduler loop.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the window and providers settle before the first check.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut ticks: u64 = 0;
        loop {
            // Mirrored library folders: at start, then every 10 minutes.
            if ticks % 20 == 0 {
                if let Ok(config_dir) = crate::settings::get_config_dir(&app) {
                    let dir = config_dir.clone();
                    let _ = tokio::task::spawn_blocking(move || super::libraries::sync_all(&dir)).await;
                    // Connector documents: the connectors answer slower, so after the folders.
                    let connectors = super::commands::registry_connectors(&app);
                    for library in super::libraries::list(&config_dir).into_iter().filter(|l| !l.connectors.is_empty()) {
                        match super::libraries::sync_connectors(&config_dir, &library.id, &connectors).await {
                            Ok(r) if r.added + r.updated + r.removed > 0 => log::info!("synced connectors of library {}: +{} ~{} -{}", library.name, r.added, r.updated, r.removed),
                            Ok(_) => {}
                            Err(e) => log::warn!("could not sync connectors of library {}: {e}", library.name),
                        }
                    }
                }
            }
            ticks += 1;
            if let Ok(config_dir) = crate::settings::get_config_dir(&app) {
                for task in take_due(&config_dir, Utc::now()) {
                    info!("running scheduled task {}", task.title);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { super::engine::run_task(app, task).await });
                }
                // The weekly knowledge review, when turned on (learning.rs).
                if super::learning::take_review(&config_dir, Utc::now()) {
                    info!("running the weekly knowledge review");
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { super::engine::run_task(app, super::learning::review_task()).await });
                }
                for task in take_new_files(&config_dir, std::time::SystemTime::now()) {
                    info!("new files started task {}", task.title);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { super::engine::run_task(app, task).await });
                }
                // Feeds: every 10 minutes (the first tick is right after start).
                if ticks % 20 == 1 {
                    for task in take_new_feed_items(&config_dir).await {
                        info!("new feed items started task {}", task.title);
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move { super::engine::run_task(app, task).await });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

/// Runs a task now, outside its schedule.
pub fn run_now(app: &AppHandle, id: &str) -> Result<(), String> {
    let config_dir = crate::settings::get_config_dir(app)?;
    let task = load(&config_dir).into_iter().find(|t| t.id == id).ok_or("No such task.")?;
    record(&config_dir, id, task.last_conversation.as_deref().unwrap_or_default(), "running");
    let app = app.clone();
    tauri::async_runtime::spawn(async move { super::engine::run_task(app, task).await });
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_calendar_lists_planned_runs_in_range() {
        let base = Task {
            id: "d".into(),
            title: "Daily".into(),
            prompt: "x".into(),
            frequency: Frequency::Daily { time: "09:00".into() },
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
        folder.frequency = Frequency::NewFiles { folder: "/tmp".into() };
        let from = Utc::now();
        let runs = occurrences(&[base, paused, folder], from, from + chrono::Duration::days(7));
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
            frequency: Frequency::NewFiles { folder: watched.path().to_string_lossy().into_owned() },
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
        assert!(take_new_files(config.path(), later).is_empty(), "the first look only notes what is there");
        crate::chat::at_rest::write(watched.path().join("march.pdf"), "x").unwrap();
        crate::chat::at_rest::write(watched.path().join(".hidden"), "x").unwrap();
        // Just written: not settled yet.
        assert!(take_new_files(config.path(), std::time::SystemTime::now()).is_empty());
        let runs = take_new_files(config.path(), later);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].prompt.contains("march.pdf") && !runs[0].prompt.contains("old.pdf") && !runs[0].prompt.contains(".hidden"));
        assert!(take_new_files(config.path(), later).is_empty(), "each file starts one run");
        assert_eq!(describe(&load(config.path())[0].frequency), format!("When new files arrive in {}", watched.path().file_name().unwrap().to_string_lossy()));
        let mut gone = load(config.path())[0].clone();
        gone.frequency = Frequency::NewFiles { folder: "/no/such/folder".into() };
        assert!(save(config.path(), gone).is_err());
    }

    use super::*;

    #[test]
    fn webhooks_get_the_answer_in_shapes_chat_apps_read() {
        let task = Task {
            id: "t1".into(),
            title: "Morning brief".into(),
            prompt: "x".into(),
            frequency: Frequency::Daily { time: "08:00".into() },
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
        let p = webhook_payload(&task, "c1", "ok", "Three items today.");
        assert_eq!((p["event"].as_str(), p["status"].as_str(), p["conversationId"].as_str()), (Some("task.finished"), Some("ok"), Some("c1")));
        assert_eq!(p["text"], "Morning brief: Three items today.");
        assert!(p["content"].as_str().unwrap().starts_with("**Morning brief**"));
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap().with_timezone(&Utc)
    }

    #[test]
    fn yearly_runs_on_its_date_and_clamps_leap_days() {
        let f = Frequency::Yearly { month: 3, day: 15, time: "10:00".into() };
        assert_eq!(next_run(&f, at(2026, 11, 17, 8, 0)).unwrap(), Some(at(2027, 3, 15, 10, 0)));
        assert_eq!(next_run(&f, at(2027, 3, 14, 8, 0)).unwrap(), Some(at(2027, 3, 15, 10, 0)));
        let leap = Frequency::Yearly { month: 2, day: 29, time: "09:00".into() };
        assert_eq!(next_run(&leap, at(2026, 11, 17, 8, 0)).unwrap(), Some(at(2027, 2, 28, 9, 0)));
        assert!(next_run(&Frequency::Yearly { month: 13, day: 1, time: "09:00".into() }, at(2026, 1, 1, 0, 0)).is_err());
        assert_eq!(describe(&f), "Every year on March 15 at 10:00");
    }

    #[test]
    fn daily_runs_today_or_tomorrow() {
        let f = Frequency::Daily { time: "09:30".into() };
        assert_eq!(next_run(&f, at(2026, 11, 17, 8, 0)).unwrap(), Some(at(2026, 11, 17, 9, 30)));
        assert_eq!(next_run(&f, at(2026, 11, 17, 9, 30)).unwrap(), Some(at(2026, 11, 18, 9, 30)));
    }

    #[test]
    fn weekly_finds_the_next_listed_day() {
        // 2026-11-17 is a Tuesday.
        let f = Frequency::Weekly { days: vec![0, 4], time: "10:00".into() };
        assert_eq!(next_run(&f, at(2026, 11, 17, 12, 0)).unwrap(), Some(at(2026, 11, 20, 10, 0)));
        assert_eq!(next_run(&f, at(2026, 11, 20, 11, 0)).unwrap(), Some(at(2026, 11, 23, 10, 0)));
        assert!(next_run(&Frequency::Weekly { days: vec![], time: "10:00".into() }, Utc::now()).is_err());
    }

    #[test]
    fn monthly_clamps_to_the_month() {
        let f = Frequency::Monthly { day: 31, time: "08:00".into() };
        assert_eq!(next_run(&f, at(2026, 11, 17, 0, 0)).unwrap(), Some(at(2026, 11, 30, 8, 0)));
    }

    #[test]
    fn once_runs_once() {
        let f = Frequency::Once { date: "2026-12-01".into(), time: "09:00".into() };
        assert_eq!(next_run(&f, at(2026, 11, 17, 0, 0)).unwrap(), Some(at(2026, 12, 1, 9, 0)));
        assert_eq!(next_run(&f, at(2026, 12, 2, 0, 0)).unwrap(), None);
        assert!(next_run(&Frequency::Daily { time: "9am".into() }, Utc::now()).is_err());
    }

    #[test]
    fn descriptions_read_naturally() {
        assert_eq!(describe(&Frequency::Weekly { days: vec![0], time: "05:30".into() }), "Every Monday at 05:30");
        assert_eq!(describe(&Frequency::Weekly { days: vec![4, 0, 2], time: "09:00".into() }), "Every Monday, Wednesday and Friday at 09:00");
        assert_eq!(describe(&Frequency::Weekly { days: vec![0, 1, 2, 3, 4], time: "09:00".into() }), "Every weekday at 09:00");
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
            frequency: Frequency::Daily { time: "09:00".into() },
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
        assert_eq!(saved.last_status.as_deref(), Some("running"));
    }

    #[test]
    fn comments_reach_the_run_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = load(dir.path());
        t.push(Task { id: "t1".into(), title: "Brief".into(), prompt: "Summarize the news.".into(), frequency: Frequency::Daily { time: "09:00".into() }, agent_id: None, model: None, enabled: true, allow_changes: false, created_at: Utc::now(), last_run: None, last_conversation: None, last_status: None, next_run: None, seen: None, reply_in: None, webhook: None, comments: Vec::new() });
        store(dir.path(), &t).unwrap();
        assert_eq!(run_prompt(&load(dir.path())[0]), "Summarize the news.");
        comment(dir.path(), "t1", Some("Skip sports."), None).unwrap();
        comment(dir.path(), "t1", Some("  Keep it to five bullets. "), None).unwrap();
        let p = run_prompt(&load(dir.path())[0]);
        assert!(p.starts_with("Summarize the news.\n\nThe user's notes") && p.contains("Skip sports.") && p.ends_with("Keep it to five bullets."), "{p}");
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
        assert_eq!(items, vec![
            FeedItem { id: "post-2".into(), title: "Rust & Tauri".into(), link: "https://b.example/2".into() },
            FeedItem { id: "https://b.example/1".into(), title: "First".into(), link: "https://b.example/1".into() },
        ]);
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>A</title><link href="https://a.example"/>
            <entry><title type="text">News &amp; notes</title><link rel="alternate" href="https://a.example/n?x=1&amp;y=2"/><id>tag:a,1</id></entry></feed>"#;
        assert_eq!(parse_feed(atom), vec![FeedItem { id: "tag:a,1".into(), title: "News & notes".into(), link: "https://a.example/n?x=1&y=2".into() }]);
        assert!(parse_feed("not a feed").is_empty());
        assert_eq!(describe(&Frequency::NewFeedItems { url: "https://blog.rust-lang.org/feed.xml".into() }), "When blog.rust-lang.org posts something new");
        assert_eq!(next_run(&Frequency::NewFeedItems { url: "https://x".into() }, Utc::now()).unwrap(), None);
    }
}
