//! Files and images the user sent in chats (Open WebUI's uploaded-file management): one list
//! across conversations, so the user can find, open or remove what they shared. A file lives in
//! its message as an `<attachment name="…">…</attachment>` block; an image as image content.
//! Removing one leaves a note in its place, so the conversation still reads.

use std::path::Path;

use chrono::{DateTime, Utc};
use rig_core::completion::message::UserContent;
use rig_core::completion::Message;
use serde::Serialize;

use super::store;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub conversation_id: String,
    pub conversation_title: String,
    pub message_index: usize,
    /// `file` or `image`.
    pub kind: &'static str,
    pub name: String,
    /// Characters of a file's text, bytes (base64) of an image.
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
}

const REMOVED: &str = "[The user removed this file.]";

/// `(name, body)` of each attachment block in `text`.
pub fn attachments(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<attachment name=\"") {
        let after = &rest[start + 18..];
        let Some(q) = after.find("\">") else { break };
        let name = after[..q].to_string();
        let body_start = &after[q + 2..];
        let Some(end) = body_start.find("</attachment>") else { break };
        out.push((name, body_start[..end].trim_matches('\n').to_string()));
        rest = &body_start[end + 13..];
    }
    out
}

/// Every upload in one conversation.
pub fn in_conversation(c: &store::Conversation) -> Vec<Upload> {
    let mut out = Vec::new();
    for (i, message) in c.messages.iter().enumerate() {
        let Message::User { content } = message else { continue };
        let at = c.times.iter().find(|(j, _)| *j == i).map(|(_, t)| *t);
        let mut images = 0;
        for part in content.iter() {
            match part {
                UserContent::Text(t) => {
                    for (name, body) in attachments(&t.text) {
                        if body.trim() == REMOVED {
                            continue;
                        }
                        out.push(Upload { conversation_id: c.id.clone(), conversation_title: c.title.clone(), message_index: i, kind: "file", name, size: body.chars().count(), at });
                    }
                }
                UserContent::Image(image) => {
                    images += 1;
                    let size = match &image.data {
                        rig_core::completion::message::DocumentSourceKind::Base64(b) => b.len(),
                        _ => 0,
                    };
                    out.push(Upload { conversation_id: c.id.clone(), conversation_title: c.title.clone(), message_index: i, kind: "image", name: format!("Image {images}"), size, at });
                }
                _ => {}
            }
        }
    }
    out
}

/// Every upload in every conversation, newest first.
pub fn list(config_dir: &Path) -> Vec<Upload> {
    let mut out: Vec<Upload> = store::list_conversations(config_dir)
        .into_iter()
        .filter_map(|s| store::load_conversation(config_dir, &s.id).ok())
        .flat_map(|c| in_conversation(&c))
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out
}

/// Takes a file (by name) or an image (`Image n`) out of a message.
pub fn remove(c: &mut store::Conversation, index: usize, kind: &str, name: &str) -> Result<(), String> {
    let Some(Message::User { content }) = c.messages.get_mut(index) else { return Err("That message no longer exists.".into()) };
    if kind == "image" {
        let n: usize = name.strip_prefix("Image ").and_then(|n| n.parse().ok()).ok_or("No such image.")?;
        let mut seen = 0;
        let position = content.iter().position(|p| {
            if matches!(p, UserContent::Image(_)) {
                seen += 1;
                seen == n
            } else {
                false
            }
        });
        let at = position.ok_or("No such image.")?;
        content[at] = UserContent::text("[The user removed an image here.]");
        return Ok(());
    }
    let open = format!("<attachment name=\"{name}\">");
    let mut found = false;
    for part in content.iter_mut() {
        if let UserContent::Text(t) = part {
            if let Some(start) = t.text.find(&open) {
                if let Some(len) = t.text[start..].find("</attachment>") {
                    let end = start + len + "</attachment>".len();
                    t.text.replace_range(start..end, &format!("{open}\n{REMOVED}\n</attachment>"));
                    found = true;
                    break;
                }
            }
        }
    }
    if found { Ok(()) } else { Err("That file is no longer in the message.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_are_listed_and_removed() {
        let mut c = store::Conversation::new("c1".into(), None, None);
        c.title = "Budget".into();
        let text = "Compare these.\n\n<attachment name=\"q1.csv\">\na,b\n1,2\n</attachment>\n\n<attachment name=\"notes.md\">\nHello\n</attachment>";
        c.messages.push(Message::User {
            content: vec![UserContent::image_base64("aGVsbG8=", Some(rig_core::completion::message::ImageMediaType::PNG), None), UserContent::text(text)],
        });
        c.times.push((0, Utc::now()));
        let all = in_conversation(&c);
        let names: Vec<(&str, &str)> = all.iter().map(|u| (u.kind, u.name.as_str())).collect();
        assert_eq!(names, [("image", "Image 1"), ("file", "q1.csv"), ("file", "notes.md")]);
        assert_eq!(all[1].size, 7);
        remove(&mut c, 0, "file", "q1.csv").unwrap();
        remove(&mut c, 0, "image", "Image 1").unwrap();
        let left: Vec<String> = in_conversation(&c).into_iter().map(|u| u.name).collect();
        assert_eq!(left, ["notes.md"]);
        assert!(remove(&mut c, 0, "file", "q1.csv").is_ok(), "removing twice leaves the note");
        assert!(remove(&mut c, 0, "file", "missing.txt").is_err());
        assert!(remove(&mut c, 5, "file", "notes.md").is_err());
        let Message::User { content } = &c.messages[0] else { panic!() };
        let all_text: String = content.iter().filter_map(|p| if let UserContent::Text(t) = p { Some(t.text.clone()) } else { None }).collect();
        assert!(all_text.contains("Compare these.") && all_text.contains(REMOVED) && all_text.contains("removed an image"));
    }
}
