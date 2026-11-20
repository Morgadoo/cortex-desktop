//! Defines schedule timing and calculates due and upcoming occurrences, including timezone
//! handling and missed-run policies for one-off, daily, weekly, and monthly schedules.

#[cfg(any(feature = "hosted", test))]
use chrono::Duration;
use chrono::{DateTime, Datelike, NaiveTime, Offset, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use croner::{errors::CronError, Cron};
use serde::{Deserialize, Serialize};

// The hosted web app's run queue. The timing engine above is also the native scheduler's
// (`chat::tasks`), so it is always built.
#[cfg(feature = "hosted")]
pub mod bridge;
#[cfg(feature = "hosted")]
pub mod coordinator;
#[cfg(feature = "hosted")]
pub mod store;

#[cfg(any(feature = "hosted", test))]
const CATCH_UP_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub timing_revision: u64,
    pub enabled: bool,
    pub timing: Timing,
    pub missed_run_policy: MissedRunPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timing {
    Once {
        at: DateTime<Utc>,
    },
    Daily {
        time: String,
        timezone: String,
    },
    /// ISO weekdays: Monday = 1, Sunday = 7. Time is exactly HH:MM.
    Weekly {
        days: Vec<u8>,
        time: String,
        timezone: String,
    },
    /// Days of the month: 1 through 31. Months without a selected day are skipped.
    Monthly {
        days: Vec<u8>,
        time: String,
        timezone: String,
    },
}

impl Timing {
    /// Whether a recurring rule keeps its wall-clock cadence when its timezone changes.
    #[cfg(feature = "hosted")]
    fn same_local_cadence(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let mut right = other.clone();
        for timing in [&mut left, &mut right] {
            match timing {
                Self::Daily { timezone, .. }
                | Self::Weekly { timezone, .. }
                | Self::Monthly { timezone, .. } => timezone.clear(),
                Self::Once { .. } => return false,
            }
        }
        left == right
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    CatchUp,
    RunOnce,
}

/// These three fields identify an occurrence across retries and redelivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg(any(feature = "hosted", test))]
pub struct Occurrence {
    pub schedule_id: String,
    pub timing_revision: u64,
    pub scheduled_for: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq)]
#[cfg(any(feature = "hosted", test))]
pub struct Plan {
    pub occurrences: Vec<Occurrence>,
    pub checked_through: DateTime<Utc>,
    /// Catch-up deliberately expires the interval up to this timestamp, exclusive.
    pub expired_before: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("schedule id must not be empty")]
    EmptyId,
    #[error("time must be HH:MM: {0}")]
    InvalidTime(String),
    #[error("unknown timezone: {0}")]
    InvalidTimezone(String),
    #[error("weekly days must be a nonempty list of unique ISO weekdays (1 through 7)")]
    InvalidWeekdays,
    #[error("monthly days must be a nonempty list of unique days (1 through 31)")]
    InvalidMonthDays,
    #[error("date is outside the supported range")]
    DateRange,
    #[error("recurrence calculation failed: {0}")]
    Recurrence(String),
}

enum Rule {
    Once(DateTime<Utc>),
    Calendar { cron: Box<Cron>, zone: Tz },
}

impl Schedule {
    fn rule(&self) -> Result<Rule, ScheduleError> {
        if self.id.trim().is_empty() {
            return Err(ScheduleError::EmptyId);
        }
        let (time, timezone, month_days, weekdays) = match &self.timing {
            Timing::Once { at } => return Ok(Rule::Once(*at)),
            Timing::Daily { time, timezone } => (time, timezone, "*".to_string(), "*".to_string()),
            Timing::Weekly {
                days,
                time,
                timezone,
            } => {
                let mut unique = days.clone();
                unique.sort_unstable();
                unique.dedup();
                if days.is_empty()
                    || unique.len() != days.len()
                    || days.iter().any(|d| !(1..=7).contains(d))
                {
                    return Err(ScheduleError::InvalidWeekdays);
                }
                (
                    time,
                    timezone,
                    "*".to_string(),
                    days.iter().map(u8::to_string).collect::<Vec<_>>().join(","),
                )
            }
            Timing::Monthly {
                days,
                time,
                timezone,
            } => {
                let mut unique = days.clone();
                unique.sort_unstable();
                unique.dedup();
                if days.is_empty()
                    || unique.len() != days.len()
                    || days.iter().any(|d| !(1..=31).contains(d))
                {
                    return Err(ScheduleError::InvalidMonthDays);
                }
                (
                    time,
                    timezone,
                    days.iter().map(u8::to_string).collect::<Vec<_>>().join(","),
                    "*".to_string(),
                )
            }
        };
        let parsed = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| ScheduleError::InvalidTime(time.clone()))?;
        if parsed.format("%H:%M").to_string() != *time {
            return Err(ScheduleError::InvalidTime(time.clone()));
        }
        let zone = timezone
            .parse()
            .map_err(|_| ScheduleError::InvalidTimezone(timezone.clone()))?;
        // Explicit seconds/minutes/hours select croner's fixed-time DST semantics.
        let expression = format!(
            "0 {} {} {} * {}",
            parsed.minute(),
            parsed.hour(),
            month_days,
            weekdays
        );
        let cron = expression.parse::<Cron>().map_err(recurrence_error)?;
        Ok(Rule::Calendar {
            cron: Box::new(cron),
            zone,
        })
    }

    /// Run-once selects the latest missed occurrence and accounts for the entire window.
    /// Catch-up returns oldest first, limited to seven days. Supported recurrences have at
    /// most one occurrence per local date, so there is no need for batch pagination yet.
    #[cfg(any(feature = "hosted", test))]
    pub fn plan(
        &self,
        checked_through: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Plan, ScheduleError> {
        let rule = self.rule()?;
        let mut plan = Plan {
            occurrences: vec![],
            checked_through: checked_through.max(now),
            expired_before: None,
        };
        if !self.enabled || now <= checked_through {
            return Ok(plan);
        }
        let run_once = matches!(self.missed_run_policy, MissedRunPolicy::RunOnce);
        let cutoff = now
            .checked_sub_signed(Duration::days(CATCH_UP_DAYS))
            .ok_or(ScheduleError::DateRange)?;
        let search_start = if run_once {
            checked_through
        } else {
            if checked_through < cutoff {
                plan.expired_before = Some(cutoff);
            }
            checked_through.max(cutoff)
        };
        let dates = match rule {
            Rule::Once(at) => {
                if at > checked_through && at <= now && (run_once || at >= cutoff) {
                    vec![at]
                } else {
                    vec![]
                }
            }
            _ if run_once => rule
                .latest_between(checked_through, now)?
                .into_iter()
                .collect(),
            _ => rule.between(search_start, now, checked_through < cutoff)?,
        };
        plan.occurrences = dates
            .into_iter()
            .map(|scheduled_for| Occurrence {
                schedule_id: self.id.clone(),
                timing_revision: self.timing_revision,
                scheduled_for,
            })
            .collect();
        Ok(plan)
    }

    /// Next occurrence strictly after `after`; disabled and exhausted one-time schedules have none.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, ScheduleError> {
        let rule = self.rule()?;
        if !self.enabled {
            return Ok(None);
        }
        match rule {
            Rule::Once(at) => Ok((at > after).then_some(at)),
            _ => rule.next_after(after).map(Some),
        }
    }
}

impl Rule {
    #[cfg(any(feature = "hosted", test))]
    fn latest_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, ScheduleError> {
        let Self::Calendar { cron, zone } = self else {
            unreachable!()
        };
        // Validate both ends even though the backward search usually examines only a few days.
        let _ = zoned(start, *zone)?;
        let end = zoned(end, *zone)?;
        let previous = cron
            .find_previous_occurrence(&end, true)
            .map_err(recurrence_error)?
            .with_timezone(&Utc);
        Ok((previous > start).then_some(previous))
    }

    fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
        let Self::Calendar { cron, zone } = self else {
            unreachable!()
        };
        cron.find_next_occurrence(&zoned(after, *zone)?, false)
            .map(|at| at.with_timezone(&Utc))
            .map_err(recurrence_error)
    }

    #[cfg(any(feature = "hosted", test))]
    fn between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        include_start: bool,
    ) -> Result<Vec<DateTime<Utc>>, ScheduleError> {
        let Self::Calendar { cron, zone } = self else {
            unreachable!()
        };
        let _ = zoned(end, *zone)?;
        let mut cursor = zoned(start, *zone)?;
        let mut inclusive = include_start;
        let mut result = vec![];
        loop {
            let next = cron
                .find_next_occurrence(&cursor, inclusive)
                .map_err(recurrence_error)?;
            if next.with_timezone(&Utc) > end {
                break;
            }
            result.push(next.with_timezone(&Utc));
            cursor = next;
            inclusive = false;
        }
        Ok(result)
    }
}

fn recurrence_error(error: CronError) -> ScheduleError {
    match error {
        CronError::TimeSearchLimitExceeded => ScheduleError::DateRange,
        _ => ScheduleError::Recurrence(error.to_string()),
    }
}

fn zoned(at: DateTime<Utc>, zone: Tz) -> Result<DateTime<Tz>, ScheduleError> {
    let utc = at.naive_utc();
    let local = utc
        .checked_add_offset(zone.offset_from_utc_datetime(&utc).fix())
        .ok_or(ScheduleError::DateRange)?;
    if !(croner::YEAR_LOWER_LIMIT..croner::YEAR_UPPER_LIMIT).contains(&local.year()) {
        return Err(ScheduleError::DateRange);
    }
    Ok(at.with_timezone(&zone))
}

#[cfg(test)]
mod tests;
