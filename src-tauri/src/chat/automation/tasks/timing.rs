//! When a task runs next, and how its schedule reads in words: times of day, weekdays, months and cron.

use super::*;
use crate::chat::error::ChatError;

pub(super) fn parse_time(time: &str) -> Result<NaiveTime, ChatError> {
    NaiveTime::parse_from_str(time.trim(), "%H:%M")
        .map_err(|_| ChatError::Invalid(format!("`{time}` is not a time like 09:30")))
}

pub(super) fn local(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

pub(super) fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next.and_then(|n| n.pred_opt()).map_or(28, |d| d.day())
}

/// The first run strictly after `after`, or `None` when there is no other.
/// The calendar rule of a time-based task in the app's one timing engine
/// ([`crate::schedules`]: time zones, daylight saving, cron), in the user's time zone. `None` for
/// what that engine does not express: yearly tasks, file and feed triggers, and monthly tasks on
/// the 29th to 31st, which run on a short month's last day instead of skipping it.
pub(super) fn calendar(frequency: &Frequency) -> Option<crate::schedules::Timing> {
    use crate::schedules::Timing;
    let timezone = iana_time_zone::get_timezone()
        .ok()
        .filter(|z| !z.is_empty())?;
    let hhmm = |time: &str| parse_time(time).ok().map(|t| t.format("%H:%M").to_string());
    match frequency {
        Frequency::Daily { time } => Some(Timing::Daily {
            time: hhmm(time)?,
            timezone,
        }),
        Frequency::Weekly { days, time } if days.iter().all(|d| *d <= 6) && !days.is_empty() => {
            let mut iso: Vec<u8> = days.iter().map(|d| d + 1).collect();
            iso.sort_unstable();
            iso.dedup();
            Some(Timing::Weekly {
                days: iso,
                time: hhmm(time)?,
                timezone,
            })
        }
        Frequency::Monthly { day, time } if (1..=28).contains(day) => Some(Timing::Monthly {
            days: vec![*day],
            time: hhmm(time)?,
            timezone,
        }),
        _ => None,
    }
}

pub(in crate::chat::automation) fn next_run(
    frequency: &Frequency,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, ChatError> {
    if let Some(timing) = calendar(frequency) {
        let schedule = crate::schedules::Schedule {
            id: "task".into(),
            timing_revision: 0,
            enabled: true,
            timing,
            missed_run_policy: crate::schedules::MissedRunPolicy::RunOnce,
        };
        // A rule the engine refuses falls through to the checks below, which say why.
        if let Ok(next) = schedule.next_after(after) {
            return Ok(next);
        }
    }
    let now = after.with_timezone(&Local);
    let today = now.date_naive();
    match frequency {
        Frequency::Once { date, time } => {
            let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|_| {
                ChatError::Invalid(format!("`{date}` is not a date like 2026-12-01"))
            })?;
            let at = local(date, parse_time(time)?);
            Ok(at.filter(|t| *t > after))
        }
        Frequency::Daily { time } => {
            let time = parse_time(time)?;
            Ok((0..3)
                .filter_map(|d| local(today + chrono::Days::new(d), time))
                .find(|t| *t > after))
        }
        Frequency::Weekly { days, time } => {
            if days.is_empty() || days.iter().any(|d| *d > 6) {
                return Err(ChatError::Invalid(
                    "Weekly tasks need days from 0 (Monday) to 6 (Sunday).".into(),
                ));
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
                return Err(ChatError::Invalid(
                    "A monthly task runs on a day from 1 to 31.".into(),
                ));
            }
            let time = parse_time(time)?;
            let mut year = today.year();
            let mut month = today.month();
            for _ in 0..14 {
                let d = u32::from(*day).min(days_in_month(year, month));
                if let Some(at) =
                    NaiveDate::from_ymd_opt(year, month, d).and_then(|date| local(date, time))
                {
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
        Frequency::Cron { expression } => next_cron(expression, after),
        Frequency::Yearly { month, day, time } => {
            if *month == 0 || *month > 12 || *day == 0 || *day > 31 {
                return Err(ChatError::Invalid(
                    "A yearly task needs a month from 1 to 12 and a day from 1 to 31.".into(),
                ));
            }
            let time = parse_time(time)?;
            for year in today.year()..=today.year() + 2 {
                let d = u32::from(*day).min(days_in_month(year, u32::from(*month)));
                if let Some(at) = NaiveDate::from_ymd_opt(year, u32::from(*month), d)
                    .and_then(|date| local(date, time))
                {
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
    const NAMES: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    match frequency {
        Frequency::Once { date, time } => format!("Once on {date} at {time}"),
        Frequency::Daily { time } => format!("Every day at {time}"),
        Frequency::Cron { expression } => format!("On the schedule `{expression}`"),
        Frequency::Weekly { days, time } => {
            let mut days = days.clone();
            days.sort_unstable();
            days.dedup();
            let names: Vec<&str> = days
                .iter()
                .filter_map(|d| NAMES.get(*d as usize).copied())
                .collect();
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
            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            let name = MONTHS
                .get(usize::from(month.saturating_sub(1)))
                .copied()
                .unwrap_or("?");
            format!("Every year on {name} {day} at {time}")
        }
        Frequency::NewFiles { folder } => {
            let name = Path::new(folder)
                .file_name()
                .map_or_else(|| folder.clone(), |n| n.to_string_lossy().into_owned());
            format!("When new files arrive in {name}")
        }
        Frequency::NewFeedItems { url } => {
            let host = url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| url.clone());
            format!("When {host} posts something new")
        }
    }
}
