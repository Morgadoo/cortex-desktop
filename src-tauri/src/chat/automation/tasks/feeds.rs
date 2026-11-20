//! Feed triggers: parsing RSS and Atom, and taking the items a task has not seen.

use super::*;

/// One item of a feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub id: String,
    pub title: String,
    pub link: String,
}

/// The text of the first `<tag>` in `xml` (CDATA and entities undone), if any.
pub(super) fn element(xml: &str, tag: &str) -> Option<String> {
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
        let raw = raw
            .strip_prefix("<![CDATA[")
            .and_then(|r| r.strip_suffix("]]>"))
            .unwrap_or(raw);
        return Some(
            raw.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&apos;", "'")
                .replace("&amp;", "&")
                .trim()
                .to_string(),
        );
    }
}

/// An attribute of the first `<tag …>` in `xml`.
pub(super) fn attribute(xml: &str, tag: &str, name: &str) -> Option<String> {
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
pub(in crate::chat::automation) fn parse_feed(xml: &str) -> Vec<FeedItem> {
    let (open, close) = if xml.contains("<entry") {
        ("<entry", "</entry>")
    } else {
        ("<item", "</item>")
    };
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let Some(len) = rest[start..].find(close) else {
            break;
        };
        let block = &rest[start..start + len];
        rest = &rest[start + len + close.len()..];
        let title = element(block, "title").unwrap_or_default();
        let link = element(block, "link")
            .filter(|l| !l.is_empty())
            .or_else(|| attribute(block, "link", "href"))
            .unwrap_or_default();
        let id = element(block, "guid")
            .or_else(|| element(block, "id"))
            .filter(|i| !i.is_empty())
            .unwrap_or_else(|| {
                if link.is_empty() {
                    title.clone()
                } else {
                    link.clone()
                }
            });
        if !id.is_empty() {
            items.push(FeedItem { id, title, link });
        }
    }
    items
}

/// Feed tasks whose feeds have new items: each with the prompt naming them. Feeds are fetched
/// at most every 10 minutes per task.
pub(in crate::chat::automation) async fn take_new_feed_items(config_dir: &Path) -> Vec<Task> {
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
    let Ok(client) = crate::support::http::client(Duration::from_secs(20)) else {
        return Vec::new();
    };
    let mut fetched = Vec::new();
    for (id, url) in due {
        match client
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
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
        let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
            continue;
        };
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let Some(seen) = task.seen.as_mut() else {
            // First look: what the feed shows now is the baseline.
            task.seen = Some(ids);
            continue;
        };
        let fresh: Vec<&FeedItem> = items
            .iter()
            .filter(|i| !seen.contains(&i.id))
            .take(20)
            .collect();
        // Remember what the feed shows now (feeds drop old items), plus anything still listed.
        let mut keep = ids.clone();
        keep.extend(seen.iter().filter(|s| !ids.contains(s)).take(200).cloned());
        *seen = keep;
        if fresh.is_empty() {
            continue;
        }
        task.last_run = Some(Utc::now());
        task.last_status = Some(TaskStatus::Running);
        let mut run = task.clone();
        let list: Vec<String> = fresh
            .iter()
            .map(|i| {
                if i.link.is_empty() {
                    format!("- {}", i.title)
                } else {
                    format!("- {} ({})", i.title, i.link)
                }
            })
            .collect();
        let url = match &task.frequency {
            Frequency::NewFeedItems { url } => url.clone(),
            _ => String::new(),
        };
        run.prompt = format!(
            "{}\n\nNew in the feed {url}:\n{}",
            task.prompt,
            list.join("\n")
        );
        started.push(run);
    }
    if let Err(e) = store(config_dir, &tasks) {
        warn!("could not save task state: {e}");
    }
    started
}
