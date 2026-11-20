//! Changing a conversation: deleting and rewriting messages, bookmarks, branches and versions of an answer.

use super::*;
use crate::support::error::StoreError;

/// Tags as stored: trimmed, lowercase, without a leading `#`, deduplicated, at most 12.
pub(in crate::chat) fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag: String = tag
            .trim()
            .trim_start_matches('#')
            .trim()
            .to_lowercase()
            .chars()
            .take(32)
            .collect();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out.truncate(12);
    out
}

pub(super) fn is_prompt(message: &Message) -> bool {
    matches!(message, Message::User { content } if content.iter().any(|c| matches!(c, UserContent::Text(_))))
}

/// Removes the message the user typed at `prompt` and the reply to it (everything up to the
/// next typed message), with any other versions of it. Refused inside the summarized part.
pub(in crate::chat) fn delete_exchange(
    conversation: &mut Conversation,
    prompt: usize,
) -> Result<(), StoreError> {
    if !conversation.messages.get(prompt).is_some_and(is_prompt) {
        return Err(StoreError::Invalid(
            "That is not a message you sent.".into(),
        ));
    }
    delete_exchanges(conversation, &[prompt]).map(|_| ())
}

/// Rewrites a message in place, without asking for a new reply: the text of a message the user
/// sent (its images kept), or the final text of an answer (the turn's tool steps kept). The
/// model reads the rewritten version from then on.
pub(in crate::chat) fn rewrite_message(
    conversation: &mut Conversation,
    index: usize,
    text: &str,
) -> Result<(), StoreError> {
    migrate(conversation);
    if text.trim().is_empty() {
        return Err(StoreError::Invalid("The message cannot be empty.".into()));
    }
    if conversation
        .summary
        .as_ref()
        .is_some_and(|s| s.upto > index)
    {
        return Err(StoreError::Invalid(
            "This part of the conversation is summarized for the model and cannot be changed."
                .into(),
        ));
    }
    match conversation.messages.get(index) {
        Some(m) if is_prompt(m) => {
            if let Some(Message::User { content }) = conversation.messages.get_mut(index) {
                let mut replaced = false;
                let items: Vec<UserContent> = std::mem::take(content);
                let kept: Vec<UserContent> = items
                    .into_iter()
                    .filter_map(|c| match c {
                        UserContent::Text(_) if replaced => None,
                        UserContent::Text(_) => {
                            replaced = true;
                            Some(UserContent::text(text))
                        }
                        other => Some(other),
                    })
                    .collect();
                *content = kept;
            }
        }
        Some(_) => {
            // An answer: its last message with text, after the prompt that asked for it.
            let start = conversation.messages[..=index]
                .iter()
                .rposition(is_prompt)
                .map_or(0, |i| i + 1);
            let at = (start..=index)
                .rev()
                .find(|i| matches!(&conversation.messages[*i], Message::Assistant(a) if a.content.iter().any(|c| matches!(c, AssistantContent::Text(t) if !t.text.trim().is_empty()))))
                .ok_or_else(|| StoreError::Invalid("This answer has no text to edit.".into()))?;
            if let Message::Assistant(assistant) = &mut conversation.messages[at] {
                let mut replaced = false;
                let items: Vec<AssistantContent> = std::mem::take(&mut assistant.content);
                let kept: Vec<AssistantContent> = items
                    .into_iter()
                    .filter_map(|c| match c {
                        AssistantContent::Text(_) if replaced => None,
                        AssistantContent::Text(_) => {
                            replaced = true;
                            Some(AssistantContent::text(text))
                        }
                        other => Some(other),
                    })
                    .collect();
                assistant.content = kept;
            }
        }
        None => return Err(StoreError::Invalid("That message no longer exists.".into())),
    }
    conversation.updated_at = Utc::now();
    Ok(())
}

/// Bookmarks or un-bookmarks the turn ending at message `index`; the snippet is the turn's text.
pub(in crate::chat) fn set_bookmark(
    conversation: &mut Conversation,
    index: usize,
    on: bool,
) -> Result<(), StoreError> {
    conversation.bookmarks.retain(|(i, _)| *i != index);
    if on {
        let turn = turns(conversation)
            .into_iter()
            .find(|t| t.index == index)
            .ok_or_else(|| StoreError::Invalid("That message no longer exists.".into()))?;
        let text: String = turn
            .parts
            .iter()
            .filter_map(|p| {
                if let Part::Text { text } = p {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        // Attached files and Markdown marks are noise in a list.
        let text = text.split("<attachment").next().unwrap_or_default();
        let flat: String = text
            .chars()
            .filter(|c| !matches!(c, '*' | '#' | '`' | '>' | '_'))
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let snippet = if flat.chars().count() > 140 {
            format!("{}…", crate::support::text::clip(&flat, 140))
        } else {
            flat
        };
        conversation.bookmarks.push((
            index,
            if snippet.is_empty() {
                format!("{} message", turn.role)
            } else {
                snippet
            },
        ));
        conversation.bookmarks.sort_by_key(|(i, _)| *i);
    }
    Ok(())
}

/// Keeps a branch of the conversation: the messages up to and including the turn ending at `upto`
/// (all when `None`), optionally without attached files and images. Notes keyed to dropped
/// messages go with them; other versions past the cut are not carried over.
pub(in crate::chat) fn branch_copy(
    conversation: &mut Conversation,
    upto: Option<usize>,
    without_files: bool,
) -> Result<(), StoreError> {
    migrate(conversation);
    if let Some(upto) = upto {
        if upto >= conversation.messages.len() {
            return Err(StoreError::Invalid("That message no longer exists.".into()));
        }
        // The turn's tool results follow its last assistant message; keep them with it.
        let mut end = upto + 1;
        while conversation.messages.get(end).is_some_and(|m| matches!(m, Message::User { content } if content.iter().all(|c| matches!(c, UserContent::ToolResult(_))))) {
            end += 1;
        }
        let len = conversation.messages.len();
        conversation.messages.truncate(end);
        if end < len {
            shift_all(&mut conversation.times, end, len);
            shift_all(&mut conversation.ratings, end, len);
            shift_all(&mut conversation.rating_reasons, end, len);
            shift_all(&mut conversation.bookmarks, end, len);
            shift_all(&mut conversation.thought, end, len);
            shift_all(&mut conversation.notices, end, len);
            conversation.branches.retain(|v| v.start() < end);
            conversation.versions = None;
            if conversation.summary.as_ref().is_some_and(|s| s.upto > end) {
                conversation.summary = None;
            }
        }
    }
    if without_files {
        for message in &mut conversation.messages {
            if let Message::User { content } = message {
                content.retain(|c| !matches!(c, UserContent::Image(_)));
                for c in content.iter_mut() {
                    if let UserContent::Text(t) = c {
                        t.text = strip_attachments(&t.text);
                    }
                }
                if content.is_empty() {
                    content.push(UserContent::text("(attachments removed)"));
                }
            }
        }
    }
    Ok(())
}

/// A message without its `<attachment …>…</attachment>` blocks.
pub(super) fn strip_attachments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<attachment name=\"") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</attachment>") {
            Some(end) => rest = &rest[start + end + "</attachment>".len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// The messages of the exchange holding message `index`: from the prompt the user typed up to the
/// next one. Tool results and follow-up answers stay with it.
pub(super) fn exchange_range(
    conversation: &Conversation,
    index: usize,
) -> Option<std::ops::Range<usize>> {
    if index >= conversation.messages.len() {
        return None;
    }
    let start = conversation.messages[..=index]
        .iter()
        .rposition(is_prompt)
        .unwrap_or(0);
    let end = conversation
        .messages
        .iter()
        .skip(start + 1)
        .position(is_prompt)
        .map_or(conversation.messages.len(), |p| start + 1 + p);
    Some(start..end)
}

/// Moves a branch point and those inside its stored branches back by `by` messages.
pub(super) fn shift_branch(point: &mut Versions, by: usize) {
    point.from -= by;
    for nested in point.nested.iter_mut().flatten() {
        shift_branch(nested, by);
    }
}

/// Deletes the exchanges holding the given messages (each turn's last message index): the prompt
/// and everything answering it, so the history still alternates as providers expect. Timestamps,
/// ratings, bookmarks and branch points after them move back; branch points inside them, and
/// the other versions they hold, go. Exchanges already summarized for the model stay.
/// Returns how many exchanges went.
pub(in crate::chat) fn delete_exchanges(
    conversation: &mut Conversation,
    indices: &[usize],
) -> Result<usize, StoreError> {
    migrate(conversation);
    let mut ranges: Vec<std::ops::Range<usize>> = indices
        .iter()
        .filter_map(|&i| exchange_range(conversation, i))
        .collect();
    if ranges.is_empty() {
        return Err(StoreError::Invalid(
            "Those messages no longer exist.".into(),
        ));
    }
    ranges.sort_by_key(|r| r.start);
    ranges.dedup();
    let count = ranges.len();
    let first = ranges[0].start;
    if conversation
        .summary
        .as_ref()
        .is_some_and(|s| s.upto > first)
    {
        return Err(StoreError::Invalid(
            "This part of the conversation is summarized for the model and cannot be removed."
                .into(),
        ));
    }
    for range in ranges.into_iter().rev() {
        let (start, end) = (range.start, range.end);
        conversation.messages.drain(start..end);
        shift_all(&mut conversation.times, start, end);
        shift_all(&mut conversation.ratings, start, end);
        shift_all(&mut conversation.rating_reasons, start, end);
        shift_all(&mut conversation.bookmarks, start, end);
        shift_all(&mut conversation.thought, start, end);
        shift_all(&mut conversation.notices, start, end);
        conversation
            .branches
            .retain(|v| !(start..end).contains(&v.start()));
        for point in conversation
            .branches
            .iter_mut()
            .filter(|v| v.start() >= end)
        {
            shift_branch(point, end - start);
        }
    }
    conversation.updated_at = Utc::now();
    Ok(count)
}

/// Drops entries keyed inside `start..end` and moves later ones back.
pub(super) fn shift_all<T>(entries: &mut Vec<(usize, T)>, start: usize, end: usize) {
    entries.retain(|(i, _)| !(start..end).contains(i));
    for (i, _) in entries.iter_mut() {
        if *i >= end {
            *i -= end - start;
        }
    }
}

/// Index of the last message the user typed.
pub(in crate::chat) fn last_prompt(conversation: &Conversation) -> Option<usize> {
    conversation.messages.iter().rposition(|m| match m {
        Message::User { content } => content.iter().any(|c| matches!(c, UserContent::Text(_))),
        _ => false,
    })
}

/// Moves a branch point saved in the old single-point format into the tree.
pub(super) fn migrate(conversation: &mut Conversation) {
    if let Some(mut v) = conversation.versions.take() {
        if v.kind.is_empty() {
            v.from += 1;
            v.kind = "answer".into();
        }
        conversation.branches.push(v);
    }
}

/// Takes the branch points that sit inside the messages from `start` on.
pub(super) fn take_inner(conversation: &mut Conversation, start: usize) -> Vec<Versions> {
    let (inner, outer): (Vec<_>, Vec<_>) = std::mem::take(&mut conversation.branches)
        .into_iter()
        .partition(|v| v.start() > start);
    conversation.branches = outer;
    inner
}

/// Keeps everything from message `from` on as a branch before it is replaced; the new
/// continuation shows last. Branch points further along go with the stored branch.
pub(in crate::chat) fn keep_branch(conversation: &mut Conversation, from: usize, kind: &str) {
    migrate(conversation);
    let tail: Vec<Message> = conversation
        .messages
        .get(from..)
        .map(<[Message]>::to_vec)
        .unwrap_or_default();
    if tail.is_empty() {
        return;
    }
    let inner = take_inner(conversation, from);
    let mut point = match conversation.branches.iter().position(|v| v.start() == from) {
        Some(i) => conversation.branches.remove(i),
        None => Versions {
            from,
            kind: kind.to_string(),
            ..Default::default()
        },
    };
    point.nested.resize(point.others.len(), Vec::new());
    let at = point.current.min(point.others.len());
    point.others.insert(at, tail);
    point.nested.insert(at, inner);
    if point.others.len() > 20 {
        point.others.remove(0);
        point.nested.remove(0);
    }
    point.current = point.others.len();
    point.kind = kind.to_string();
    conversation.branches.push(point);
    conversation.branches.sort_by_key(Versions::start);
}

/// Shows branch `target` (0-based) at the branch point that starts at message `from`.
/// The answer text of every version at a branch point, in switcher order, to compare them
/// (Open WebUI's Compare Text).
pub(in crate::chat) fn version_texts(
    conversation: &Conversation,
    from: usize,
) -> Result<Vec<String>, StoreError> {
    let mut conversation = conversation.clone();
    migrate(&mut conversation);
    let point = conversation
        .branches
        .iter()
        .find(|v| v.start() == from)
        .ok_or_else(|| StoreError::Invalid("There are no other versions here.".into()))?;
    let start = point.start();
    let live: Vec<Message> = conversation
        .messages
        .get(start..)
        .map(<[Message]>::to_vec)
        .unwrap_or_default();
    let mut all: Vec<Vec<Message>> = point.others.clone();
    all.insert(point.current.min(all.len()), live);
    Ok(all
        .iter()
        .map(|messages| {
            // The reply: assistant text up to the next message the user typed.
            let skip = usize::from(messages.first().is_some_and(is_prompt));
            messages
                .iter()
                .skip(skip)
                .take_while(|m| !is_prompt(m))
                .filter_map(|m| match m {
                    Message::Assistant(a) => Some(
                        a.content
                            .iter()
                            .filter_map(|c| {
                                if let AssistantContent::Text(t) = c {
                                    Some(t.text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    _ => None,
                })
                .filter(|t| !t.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .collect())
}

pub(in crate::chat) fn select_version(
    conversation: &mut Conversation,
    from: usize,
    target: usize,
) -> Result<(), StoreError> {
    migrate(conversation);
    let index = conversation
        .branches
        .iter()
        .position(|v| v.start() == from)
        .ok_or_else(|| StoreError::Invalid("There are no other versions here.".into()))?;
    let start = conversation.branches[index].start();
    if start > conversation.messages.len() || target > conversation.branches[index].others.len() {
        return Err(StoreError::Invalid("There is no such version.".into()));
    }
    let mut point = conversation.branches.remove(index);
    point.nested.resize(point.others.len(), Vec::new());
    let live = conversation.messages.split_off(start);
    let live_inner = take_inner(conversation, start);
    let at = point.current.min(point.others.len());
    point.others.insert(at, live);
    point.nested.insert(at, live_inner);
    let chosen = point.others.remove(target);
    let chosen_inner = point.nested.remove(target);
    conversation.messages.extend(chosen);
    conversation.branches.extend(chosen_inner);
    conversation.times.retain(|(i, _)| *i < start);
    conversation.thought.retain(|(i, _)| *i < start);
    conversation.notices.retain(|(i, _)| *i < start);
    conversation.ratings.retain(|(i, _)| *i < start);
    if conversation
        .summary
        .as_ref()
        .is_some_and(|s| s.upto > start)
    {
        conversation.summary = None;
    }
    point.current = target;
    conversation.branches.push(point);
    conversation.branches.sort_by_key(Versions::start);
    Ok(())
}
