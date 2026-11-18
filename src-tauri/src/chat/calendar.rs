//! Calendars and contacts (Osaurus's Calendar and Contacts integrations), read from the formats
//! every app exports: a calendar's private ICS link or an `.ics` file (Google, Proton, Outlook,
//! iCloud, Thunderbird), and `.vcf` contact files. The assistant reads them with
//! `calendar_events` and `find_contact`; nothing is written back.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub name: String,
    /// An `https://` or `webcal://` link, or a file on this computer.
    pub location: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub calendars: Vec<Source>,
    pub contacts: Vec<Source>,
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("calendars.json")
}

pub fn load(config_dir: &Path) -> Settings {
    crate::chat::at_rest::read_to_string(path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    write_json(&path(config_dir), settings)
}

/// The text of a source: fetched for a link, read for a file.
pub async fn fetch(location: &str) -> Result<String, String> {
    let location = location.trim();
    if let Some(rest) = location.strip_prefix("webcal://") {
        return fetch_url(&format!("https://{rest}")).await;
    }
    if location.starts_with("https://") || location.starts_with("http://") {
        return fetch_url(location).await;
    }
    let path = location.strip_prefix("file://").unwrap_or(location);
    crate::chat::at_rest::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

async fn fetch_url(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let response = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("the calendar link answered {}", response.status()));
    }
    response.text().await.map_err(|e| e.to_string())
}

/// Lines with folded continuations joined (RFC 5545 §3.1), as `NAME;PARAMS` and value.
fn properties(text: &str) -> Vec<(String, String)> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
        .into_iter()
        .filter_map(|l| {
            let (head, value) = l.split_once(':')?;
            Some((head.to_string(), value.to_string()))
        })
        .collect()
}

fn unescape(value: &str) -> String {
    value.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

fn param<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    head.split(';').skip(1).find_map(|p| p.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.trim_matches('"')))
}

/// When an event starts or ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum When {
    Day(NaiveDate),
    At(DateTime<Utc>),
}

impl When {
    fn instant(self) -> DateTime<Utc> {
        match self {
            When::At(t) => t,
            When::Day(d) => d.and_hms_opt(0, 0, 0).map_or_else(Utc::now, |n| n.and_utc()),
        }
    }
}

fn parse_when(head: &str, value: &str) -> Option<When> {
    let value = value.trim();
    if value.len() == 8 || param(head, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) {
        return NaiveDate::parse_from_str(&value[..8.min(value.len())], "%Y%m%d").ok().map(When::Day);
    }
    let (stamp, utc) = match value.strip_suffix('Z') {
        Some(s) => (s, true),
        None => (value, false),
    };
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S").ok()?;
    if utc {
        return Some(When::At(naive.and_utc()));
    }
    if let Some(tz) = param(head, "TZID").and_then(|t| t.parse::<chrono_tz::Tz>().ok()) {
        return tz.from_local_datetime(&naive).earliest().map(|t| When::At(t.with_timezone(&Utc)));
    }
    // A floating time: the computer's own zone.
    chrono::Local.from_local_datetime(&naive).earliest().map(|t| When::At(t.with_timezone(&Utc)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub title: String,
    pub start: When,
    pub end: Option<When>,
    pub location: String,
    pub description: String,
    pub rrule: Option<String>,
    pub exdates: Vec<DateTime<Utc>>,
}

/// The events of an ICS calendar.
pub fn parse_ics(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut current: Option<Event> = None;
    for (head, value) in properties(text) {
        let name = head.split(';').next().unwrap_or_default().to_ascii_uppercase();
        match (name.as_str(), value.trim()) {
            ("BEGIN", "VEVENT") => current = Some(Event { title: String::new(), start: When::Day(NaiveDate::MIN), end: None, location: String::new(), description: String::new(), rrule: None, exdates: Vec::new() }),
            ("END", "VEVENT") => {
                if let Some(e) = current.take().filter(|e| e.start != When::Day(NaiveDate::MIN)) {
                    out.push(e);
                }
            }
            _ => {
                let Some(e) = current.as_mut() else { continue };
                match name.as_str() {
                    "SUMMARY" => e.title = unescape(&value),
                    "LOCATION" => e.location = unescape(&value),
                    "DESCRIPTION" => e.description = unescape(&value),
                    "DTSTART" => {
                        if let Some(w) = parse_when(&head, &value) {
                            e.start = w;
                        }
                    }
                    "DTEND" => e.end = parse_when(&head, &value),
                    "RRULE" => e.rrule = Some(value.trim().to_string()),
                    "EXDATE" => e.exdates.extend(value.split(',').filter_map(|v| parse_when(&head, v)).map(When::instant)),
                    _ => {}
                }
            }
        }
    }
    out
}

/// One occurrence of an event in a window, in local time.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub title: String,
    pub calendar: String,
    /// `2026-11-20 14:00` in local time, or `2026-11-20` for a day-long event.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub location: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(skip)]
    pub at: DateTime<Utc>,
}

fn add_months(t: DateTime<Utc>, months: i64) -> Option<DateTime<Utc>> {
    let total = t.year() as i64 * 12 + t.month0() as i64 + months;
    let (year, month) = ((total / 12) as i32, (total % 12) as u32 + 1);
    let date = NaiveDate::from_ymd_opt(year, month, t.day())?;
    Some(date.and_time(t.time()).and_utc())
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code.get(code.len().saturating_sub(2)..)? {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// The start times of an event from `from` to `to`, with its repeat rule (daily, weekly with
/// days, monthly, yearly; interval, count, until; exceptions). At most `limit`.
pub fn starts(event: &Event, from: DateTime<Utc>, to: DateTime<Utc>, limit: usize) -> Vec<DateTime<Utc>> {
    let first = event.start.instant();
    let Some(rule) = &event.rrule else {
        let end = event.end.map_or(first, When::instant);
        return if first <= to && end.max(first) >= from { vec![first] } else { Vec::new() };
    };
    let part = |key: &str| rule.split(';').find_map(|p| p.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.to_string()));
    let interval = part("INTERVAL").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let count = part("COUNT").and_then(|v| v.parse::<usize>().ok());
    let until = part("UNTIL").and_then(|v| parse_when("", &v)).map(When::instant);
    let by_day: Vec<Weekday> = part("BYDAY").map(|v| v.split(',').filter_map(weekday).collect()).unwrap_or_default();
    let freq = part("FREQ").unwrap_or_default().to_ascii_uppercase();
    let mut out = Vec::new();
    let mut produced = 0usize;
    let mut step = 0i64;
    // Walk the series from its start; stop past the window, the count, or a safety bound.
    while step < 5_000 {
        let base = match freq.as_str() {
            "DAILY" => Some(first + Duration::days(step * interval)),
            "WEEKLY" => Some(first + Duration::weeks(step * interval)),
            "MONTHLY" => add_months(first, step * interval),
            "YEARLY" => add_months(first, step * interval * 12),
            _ => None,
        };
        let Some(base) = base else {
            if freq.is_empty() || !matches!(freq.as_str(), "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") {
                break;
            }
            step += 1;
            continue;
        };
        let candidates: Vec<DateTime<Utc>> = if freq == "WEEKLY" && !by_day.is_empty() {
            let monday = base - Duration::days(base.weekday().num_days_from_monday() as i64);
            let mut days: Vec<DateTime<Utc>> = by_day.iter().map(|d| monday + Duration::days(d.num_days_from_monday() as i64)).filter(|t| *t >= first).collect();
            days.sort();
            days
        } else {
            vec![base]
        };
        for t in candidates {
            if until.is_some_and(|u| t > u) || count.is_some_and(|c| produced >= c) || t > to {
                return out;
            }
            produced += 1;
            if t >= from && !event.exdates.contains(&t) {
                out.push(t);
                if out.len() >= limit {
                    return out;
                }
            }
        }
        step += 1;
    }
    out
}

/// The occurrences of every event between `from` and `to`, soonest first.
pub fn occurrences(calendar: &str, events: &[Event], from: DateTime<Utc>, to: DateTime<Utc>, query: &str) -> Vec<Occurrence> {
    let query = query.trim().to_lowercase();
    let mut out = Vec::new();
    for event in events {
        if !query.is_empty() && !format!("{} {} {}", event.title, event.location, event.description).to_lowercase().contains(&query) {
            continue;
        }
        let length = event.end.map_or(Duration::zero(), |e| e.instant() - event.start.instant());
        let all_day = matches!(event.start, When::Day(_));
        for start in starts(event, from, to, 200) {
            let end = start + length;
            let fmt = |t: DateTime<Utc>| if all_day { t.date_naive().format("%Y-%m-%d").to_string() } else { t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string() };
            out.push(Occurrence {
                title: event.title.clone(),
                calendar: calendar.to_string(),
                start: fmt(start),
                // A day-long event's end is the day after its last day.
                end: if all_day { fmt(end - Duration::days(1)) } else { fmt(end) },
                all_day,
                location: event.location.clone(),
                notes: event.description.chars().take(300).collect(),
                at: start,
            });
        }
    }
    out.sort_by_key(|o| o.at);
    out
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub organization: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub birthday: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The contacts of a vCard file.
pub fn parse_vcards(text: &str) -> Vec<Contact> {
    let mut out = Vec::new();
    let mut current: Option<Contact> = None;
    for (head, value) in properties(text) {
        // `item1.EMAIL` (Apple) is an EMAIL.
        let name = head.split(';').next().unwrap_or_default().rsplit('.').next().unwrap_or_default().to_ascii_uppercase();
        let value = unescape(value.trim());
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => current = Some(Contact::default()),
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                if let Some(c) = current.take().filter(|c| !c.name.is_empty() || !c.emails.is_empty()) {
                    out.push(c);
                }
            }
            _ => {
                let Some(c) = current.as_mut() else { continue };
                match name.as_str() {
                    "FN" => c.name = value,
                    "N" if c.name.is_empty() => {
                        let parts: Vec<&str> = value.split(';').collect();
                        c.name = format!("{} {}", parts.get(1).unwrap_or(&""), parts.first().unwrap_or(&"")).trim().to_string();
                    }
                    "EMAIL" => c.emails.push(value),
                    "TEL" => c.phones.push(value),
                    "ORG" => c.organization = value.replace(';', ", ").trim_matches([',', ' ']).to_string(),
                    "TITLE" | "ROLE" => c.role = value,
                    "BDAY" => c.birthday = value,
                    "ADR" => c.address = value.split(';').filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(", "),
                    "NOTE" => c.note = value.chars().take(300).collect(),
                    _ => {}
                }
            }
        }
    }
    out
}

/// Contacts whose details hold every word of the query.
pub fn find(contacts: &[Contact], query: &str) -> Vec<Contact> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    contacts
        .iter()
        .filter(|c| {
            let hay = format!("{} {} {} {} {} {}", c.name, c.emails.join(" "), c.phones.join(" "), c.organization, c.role, c.note).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .take(10)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nSUMMARY:Dentist\\, Dr. Lima\r\nDTSTART:20261120T100000Z\r\nDTEND:20261120T110000Z\r\nLOCATION:Rua Augusta 10\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Team stand-up\r\nDTSTART;TZID=Europe/Lisbon:20261102T093000\r\nDTEND;TZID=Europe/Lisbon:20261102T094500\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nEXDATE;TZID=Europe/Lisbon:20261120T093000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261208\r\nDTEND;VALUE=DATE:20261209\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Rent\r\nDTSTART:20260105T090000Z\r\nRRULE:FREQ=MONTHLY;COUNT=3\r\nDESCRIPTION:Pay the rent\r\n  to Ana\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn events_repeat_and_skip_exceptions() {
        let events = parse_ics(ICS);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].title, "Dentist, Dr. Lima");
        assert_eq!(events[3].description, "Pay the rent to Ana");
        // The week of 16 November: stand-up Monday, Wednesday, Friday (Friday excepted), the dentist on Friday.
        let week = occurrences("Work", &events, day(2026, 11, 16), day(2026, 11, 22), "");
        let titles: Vec<&str> = week.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(titles, ["Team stand-up", "Team stand-up", "Dentist, Dr. Lima"]);
        // 09:30 in Lisbon is 09:30 UTC in winter.
        assert_eq!(week[0].at, Utc.with_ymd_and_hms(2026, 11, 16, 9, 30, 0).unwrap());
        // A day-long event shows its own day as start and end.
        let holiday = occurrences("Home", &events, day(2026, 12, 1), day(2026, 12, 31), "holiday");
        assert_eq!((holiday[0].start.as_str(), holiday[0].end.as_str(), holiday[0].all_day), ("2026-12-08", "2026-12-08", true));
        // COUNT stops the series.
        let rent = occurrences("Home", &events, day(2026, 1, 1), day(2026, 12, 31), "rent");
        assert_eq!(rent.len(), 3);
        assert_eq!(rent[2].at, Utc.with_ymd_and_hms(2026, 3, 5, 9, 0, 0).unwrap());
    }

    #[test]
    fn contacts_are_read_and_found() {
        let vcf = "BEGIN:VCARD\nVERSION:3.0\nFN:Rita Duarte\nORG:Acme Corp;Sales\nTITLE:Account manager\nitem1.EMAIL;TYPE=INTERNET:rita@acme.example\nTEL;TYPE=CELL:+351 912 345 678\nBDAY:1988-04-14\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nN:Silva;Marta;;;\nEMAIL:marta@example.org\nEND:VCARD\n";
        let contacts = parse_vcards(vcf);
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].organization, "Acme Corp, Sales");
        assert_eq!(contacts[0].emails, ["rita@acme.example"]);
        assert_eq!(contacts[1].name, "Marta Silva");
        assert_eq!(find(&contacts, "acme")[0].name, "Rita Duarte");
        assert_eq!(find(&contacts, "marta silva").len(), 1);
        assert!(find(&contacts, "nobody").is_empty());
    }
}
