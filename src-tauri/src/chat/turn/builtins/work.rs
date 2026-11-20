//! The built-ins that act: saving and using skills, calendars and contacts, libraries, the plan, and scheduling tasks.

use super::*;

/// `SAVE_SKILL`.
pub(super) async fn save_skill(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let provenance = skill_provenance(config_dir, &str_arg("name"), &str_arg("conversation"));
    match skills::save(
        config_dir,
        &str_arg("name"),
        &str_arg("description"),
        &str_arg("instructions"),
    ) {
        Ok(()) => {
            crate::chat::traces::register(
                config_dir,
                "skill",
                &str_arg("name"),
                Some(&crate::chat::traces::hash(&str_arg("instructions"))),
                Some(&provenance),
                "active",
            );
            ok(format!(
                "Saved the skill `{}`. The user can call it with /{}.",
                str_arg("name"),
                str_arg("name")
            ))
        }
        Err(e) => err(e),
    }
}

/// `CALENDAR_EVENTS`.
pub(super) async fn calendar_events(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let today = chrono::Local::now().date_naive();
    let date = |key: &str| chrono::NaiveDate::parse_from_str(&str_arg(key), "%Y-%m-%d").ok();
    let from_day = date("from").unwrap_or(today);
    let to_day = date("to")
        .unwrap_or(from_day + chrono::Duration::days(7))
        .max(from_day);
    if (to_day - from_day).num_days() > 366 {
        return err("Ask for at most a year at a time.");
    }
    let local = |d: chrono::NaiveDate, end: bool| {
        use chrono::TimeZone as _;
        let t = if end {
            d.and_hms_opt(23, 59, 59)
        } else {
            d.and_hms_opt(0, 0, 0)
        }
        .unwrap_or_default();
        chrono::Local
            .from_local_datetime(&t)
            .earliest()
            .map_or_else(chrono::Utc::now, |t| t.with_timezone(&chrono::Utc))
    };
    let (from, to) = (local(from_day, false), local(to_day, true));
    let mut events = Vec::new();
    let mut problems = Vec::new();
    for cal in crate::chat::calendar::load(config_dir).calendars {
        match crate::chat::calendar::fetch(&cal.location).await {
            Ok(text) => events.extend(crate::chat::calendar::occurrences(
                &cal.name,
                &crate::chat::calendar::parse_ics(&text),
                from,
                to,
                &str_arg("query"),
            )),
            Err(e) => problems.push(format!("{}: {e}", cal.name)),
        }
    }
    events.sort_by_key(|e| e.at);
    let shown: Vec<_> = events.iter().take(80).collect();
    ok(serde_json::to_string_pretty(&json!({
        "from": from_day.to_string(),
        "to": to_day.to_string(),
        "events": shown,
        "more": events.len().saturating_sub(shown.len()),
        "unreadable": problems,
    }))
    .unwrap_or_default())
}

/// `FIND_CONTACT`.
pub(super) async fn find_contact(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a name, email or company to look up.");
    }
    let mut all = Vec::new();
    let mut problems = Vec::new();
    for book in crate::chat::calendar::load(config_dir).contacts {
        match crate::chat::calendar::fetch(&book.location).await {
            Ok(text) => all.extend(crate::chat::calendar::parse_vcards(&text)),
            Err(e) => problems.push(format!("{}: {e}", book.name)),
        }
    }
    let found = crate::chat::calendar::find(&all, &query);
    ok(serde_json::to_string_pretty(
        &json!({ "query": query, "contacts": found, "unreadable": problems }),
    )
    .unwrap_or_default())
}

/// `SEARCH_LIBRARY`.
pub(super) async fn search_library(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a search query.");
    }
    let wanted = str_arg("library");
    let only: Vec<String> = if wanted.is_empty() {
        Vec::new()
    } else {
        match crate::chat::libraries::list(config_dir)
            .into_iter()
            .find(|l| l.name.eq_ignore_ascii_case(&wanted))
        {
            Some(l) => vec![l.id],
            None => return err(format!("There is no library named {wanted}.")),
        }
    };
    let since_text = str_arg("since");
    let since = crate::chat::libraries::parse_since(&since_text, chrono::Utc::now());
    if !since_text.is_empty() && since.is_none() {
        return err("`since` is a date like 2026-03-01, or a span like 30d, 6m or 1y.");
    }
    // With a cutoff, a wider pool is searched and the older documents' passages left out.
    let (hits, note) = crate::chat::embeddings::search(
        config_dir,
        &only,
        &query,
        if since.is_some() { 30 } else { 6 },
    )
    .await;
    let hits: Vec<_> = match since {
        Some(cutoff) => crate::chat::libraries::changed_since(config_dir, hits, cutoff)
            .into_iter()
            .take(6)
            .collect(),
        None => hits,
    };
    let note = match (since, hits.is_empty()) {
        (Some(cutoff), true) => Some(format!(
            "No document changed since {} matched; search without `since` to include older ones.",
            cutoff.format("%Y-%m-%d")
        )),
        _ => note,
    };
    if hits.is_empty() {
        ok(json!({ "query": query, "passages": [], "note": note.unwrap_or_else(|| "Nothing matched; try other keywords.".into()) }).to_string())
    } else {
        ok(
            serde_json::to_string_pretty(
                &json!({ "query": query, "passages": hits, "note": note }),
            )
            .unwrap_or_default(),
        )
    }
}

/// `UPDATE_PLAN`.
pub(super) async fn update_plan(_config_dir: &Path, args: &Value) -> Outcome {
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if steps == 0 {
        err("Give the plan's steps.")
    } else {
        ok(format!("Plan shown to the user ({steps} steps)."))
    }
}

/// `USE_SKILL`.
pub(super) async fn use_skill(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    match skills::find(config_dir, &str_arg("name"))
        .inspect(|skill| crate::chat::traces::used(config_dir, "skill", &skill.name))
    {
        Some(skill) if skill.enabled && !str_arg("file").trim().is_empty() => {
            match skills::read_file(config_dir, &skill.name, str_arg("file").trim()) {
                Ok(text) => ok(text),
                Err(e) => err(format!(
                    "{e} Its files: {}.",
                    if skill.files.is_empty() {
                        "none".to_string()
                    } else {
                        skill.files.join(", ")
                    }
                )),
            }
        }
        Some(skill) if skill.enabled => ok(skills::loaded(&skill)),
        Some(_) => err("That skill is turned off."),
        None => {
            // Not a name: search the skills with the words given.
            let all = skills::list(config_dir);
            let enabled: Vec<&skills::Skill> = all.iter().filter(|s| s.enabled).collect();
            let found: Vec<String> = skills::relevant(&enabled, &str_arg("name"))
                .into_iter()
                .take(5)
                .map(|s| format!("- {}: {}", s.name, s.description))
                .collect();
            if found.is_empty() {
                err("There is no skill with that name, and none matches those words.")
            } else {
                ok(format!(
                    "No skill has that exact name. These match; call use_skill again with one \
                     of their names:\n{}",
                    found.join("\n")
                ))
            }
        }
    }
}

/// Who a skill counts as made by, for the lifecycle: a skill the user wrote stays theirs when the
/// assistant updates it, so it is never retired as the assistant's; a new one is the assistant's,
/// with the chat it came from.
pub(crate) fn skill_provenance(config_dir: &Path, name: &str, conversation: &str) -> String {
    let users = skills::find(config_dir, name).is_some_and(|s| s.origin == "user")
        && crate::chat::traces::artifact(config_dir, "skill", name)
            .is_none_or(|a| a.provenance.as_deref().is_none_or(|p| p == "user"));
    if users {
        "user".into()
    } else if conversation.is_empty() {
        "assistant".into()
    } else {
        conversation.into()
    }
}

pub(super) fn schedule(config_dir: &Path, args: &Value) -> Outcome {
    use crate::chat::tasks::{self, Frequency};
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let time = s("time");
    let frequency = match s("frequency").as_str() {
        "once" => Frequency::Once {
            date: s("date"),
            time,
        },
        "daily" => Frequency::Daily { time },
        "weekly" => {
            let days: Vec<u8> = args
                .get("days")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(tasks::weekday_index)
                        .collect()
                })
                .unwrap_or_default();
            Frequency::Weekly { days, time }
        }
        "monthly" => Frequency::Monthly {
            day: args
                .get("day_of_month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .min(31) as u8,
            time,
        },
        "yearly" => Frequency::Yearly {
            month: args
                .get("month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 12) as u8,
            day: args
                .get("day_of_month")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 31) as u8,
            time,
        },
        "cron" => Frequency::Cron {
            expression: s("cron"),
        },
        other => return err(format!("Unknown frequency `{other}`.")),
    };
    let agent_id = match s("agent") {
        name if name.is_empty() => None,
        name => match crate::chat::agents::load(config_dir)
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
        {
            Some(agent) => Some(agent.id),
            None => return err(format!("There is no agent named {name}.")),
        },
    };
    // The engine fills `reply_in` with this conversation's id when the model asked for it.
    let reply_in = args
        .get("reply_in")
        .and_then(Value::as_str)
        .filter(|_| args.get("in_this_chat").and_then(Value::as_bool) == Some(true))
        .map(str::to_string);
    let task = tasks::Task {
        id: uuid::Uuid::new_v4().to_string(),
        title: s("title"),
        prompt: s("prompt"),
        frequency: frequency.clone(),
        agent_id,
        model: None,
        enabled: true,
        allow_changes: false,
        created_at: chrono::Utc::now(),
        last_run: None,
        last_conversation: None,
        last_status: None,
        next_run: None,
        seen: None,
        reply_in,
        webhook: None,
        comments: Vec::new(),
    };
    let id = task.id.clone();
    match tasks::save(config_dir, task) {
        Ok(saved) => {
            let next = saved.iter().find(|t| t.id == id).and_then(|t| t.next_run);
            ok(json!({
                "scheduled": tasks::describe(&frequency),
                "next_run": next.map(|n| n.with_timezone(&chrono::Local).format("%A %-d %B %Y, %H:%M").to_string()),
                "note": "Tell the user it is scheduled; they can pause or edit it on the Scheduled page."
            })
            .to_string())
        }
        Err(e) => err(e),
    }
}
