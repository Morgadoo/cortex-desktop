//! Importing conversations from other assistants' data exports.
//!
//! ChatGPT and Claude both export a `conversations.json` (inside a zip, or on its own). ChatGPT
//! stores each conversation as a tree of message nodes; the conversation as the user last saw it
//! is the path from the root to `current_node`. Claude stores a flat `chat_messages` list. Only
//! the text of user and assistant messages comes across; tool calls, system messages and
//! attachments do not.

use std::io::Read;
use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use rig_core::completion::message::Message;
use serde_json::Value;

use super::store::{self, Conversation};

#[derive(Debug, Default, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub source: String,
}

/// Reads `conversations.json` from an export zip, or the JSON file itself.
fn read_export(path: &Path) -> Result<Value, String> {
    let bytes = crate::chat::at_rest::read(path).map_err(|e| e.to_string())?;
    let json = if bytes.starts_with(b"PK") {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("Not a readable zip: {e}"))?;
        let name = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .find(|n| n.rsplit('/').next() == Some("conversations.json"))
            .ok_or("The export has no conversations.json.")?;
        let mut text = String::new();
        archive.by_name(&name).map_err(|e| e.to_string())?.read_to_string(&mut text).map_err(|e| e.to_string())?;
        text
    } else {
        String::from_utf8(bytes).map_err(|_| "The file is not text.".to_string())?
    };
    serde_json::from_str(&json).map_err(|e| format!("The export is not valid JSON: {e}"))
}

fn time_from(value: Option<&Value>) -> Option<DateTime<Utc>> {
    match value? {
        Value::Number(n) => n.as_f64().and_then(|s| Utc.timestamp_millis_opt((s * 1000.0) as i64).single()),
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

/// One imported message: who said it, what, and when.
struct Line {
    user: bool,
    text: String,
    at: Option<DateTime<Utc>>,
}

/// The visible thread of one ChatGPT conversation: root to `current_node`.
fn chatgpt_lines(conversation: &Value) -> Vec<Line> {
    let Some(mapping) = conversation.get("mapping").and_then(Value::as_object) else { return Vec::new() };
    let mut node = conversation
        .get("current_node")
        .and_then(Value::as_str)
        .map(str::to_string)
        // Without a current node, follow the last child from the root.
        .or_else(|| mapping.iter().find(|(_, n)| n.get("parent").is_none_or(Value::is_null)).map(|(k, _)| k.clone()));
    let mut path = Vec::new();
    let mut guard = 0;
    while let Some(id) = node.take() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        let Some(entry) = mapping.get(&id) else { break };
        path.push(entry);
        node = entry.get("parent").and_then(Value::as_str).map(str::to_string);
    }
    path.reverse();
    path.into_iter()
        .filter_map(|entry| {
            let message = entry.get("message")?;
            let role = message.pointer("/author/role")?.as_str()?;
            if role != "user" && role != "assistant" {
                return None;
            }
            if message.pointer("/metadata/is_visually_hidden_from_conversation").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let content = message.get("content")?;
            let kind = content.get("content_type").and_then(Value::as_str).unwrap_or("text");
            if kind != "text" && kind != "multimodal_text" {
                return None;
            }
            let text = content
                .get("parts")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            let text = text.trim().to_string();
            (!text.is_empty()).then(|| Line { user: role == "user", text, at: time_from(message.get("create_time")) })
        })
        .collect()
}

fn claude_lines(conversation: &Value) -> Vec<Line> {
    conversation
        .get("chat_messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let sender = m.get("sender").and_then(Value::as_str)?;
            let mut text = m.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if text.is_empty() {
                text = m
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string();
            }
            (!text.is_empty() && (sender == "human" || sender == "assistant"))
                .then(|| Line { user: sender == "human", text, at: time_from(m.get("created_at")) })
        })
        .collect()
}

/// Builds conversations from an export's JSON, without saving them.
fn convert(export: &Value) -> Result<(Vec<Conversation>, usize, &'static str), String> {
    let items = export.as_array().ok_or("The export is not a list of conversations.")?;
    let source = if items.iter().any(|c| c.get("mapping").is_some()) {
        "ChatGPT"
    } else if items.iter().any(|c| c.get("chat_messages").is_some()) {
        "Claude"
    } else {
        return Err("This does not look like a ChatGPT or Claude export.".into());
    };
    let mut out = Vec::new();
    let mut skipped = 0;
    for item in items {
        let lines = if source == "ChatGPT" { chatgpt_lines(item) } else { claude_lines(item) };
        if lines.is_empty() {
            skipped += 1;
            continue;
        }
        let mut conversation = Conversation::new(uuid::Uuid::new_v4().to_string(), None, None);
        conversation.title = item
            .get("title")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("Imported chat")
            .chars()
            .take(120)
            .collect();
        let created = time_from(item.get("create_time").or_else(|| item.get("created_at"))).or(lines[0].at);
        let updated = time_from(item.get("update_time").or_else(|| item.get("updated_at"))).or(lines.last().and_then(|l| l.at));
        for line in lines {
            conversation.messages.push(if line.user { Message::user(line.text) } else { Message::assistant(line.text) });
            if let Some(at) = line.at {
                conversation.times.push((conversation.messages.len() - 1, at));
            }
        }
        if let Some(created) = created {
            conversation.created_at = created;
        }
        conversation.updated_at = updated.unwrap_or(conversation.created_at);
        out.push(conversation);
    }
    Ok((out, skipped, source))
}

/// Imports every conversation in the export at `path`.
pub fn import(config_dir: &Path, path: &Path) -> Result<ImportReport, String> {
    let (conversations, skipped, source) = convert(&read_export(path)?)?;
    let imported = conversations.len();
    for conversation in &conversations {
        store::save_conversation(config_dir, conversation)?;
    }
    Ok(ImportReport { imported, skipped, source: source.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn imports_the_visible_chatgpt_thread() {
        // An edited prompt leaves a dead branch (b1); the visible thread is root → a → b2 → c.
        let export = json!([{
            "title": "Trip ideas",
            "create_time": 1700000000.0,
            "update_time": 1700000100.0,
            "current_node": "c",
            "mapping": {
                "root": { "message": null, "parent": null, "children": ["s"] },
                "s": { "parent": "root", "message": { "author": { "role": "system" }, "content": { "content_type": "text", "parts": [""] } } },
                "a": { "parent": "s", "message": { "author": { "role": "user" }, "create_time": 1700000001.0, "content": { "content_type": "text", "parts": ["Where to go in May?"] } } },
                "b1": { "parent": "a", "message": { "author": { "role": "assistant" }, "content": { "content_type": "text", "parts": ["Old answer"] } } },
                "b2": { "parent": "a", "message": { "author": { "role": "assistant" }, "content": { "content_type": "text", "parts": ["Try Porto."] } } },
                "t": { "parent": "b2", "message": { "author": { "role": "tool" }, "content": { "content_type": "text", "parts": ["search"] } } },
                "c": { "parent": "t", "message": { "author": { "role": "user" }, "content": { "content_type": "text", "parts": ["Thanks!"] } } }
            }
        }, { "title": "Empty", "mapping": {} }]);
        let (conversations, skipped, source) = convert(&export).unwrap();
        assert_eq!(source, "ChatGPT");
        assert_eq!(skipped, 1);
        let c = &conversations[0];
        assert_eq!(c.title, "Trip ideas");
        let turns = store::turns(c);
        let texts: Vec<String> = turns.iter().map(|t| serde_json::to_string(&t.parts).unwrap()).collect();
        assert_eq!(turns.len(), 3, "{texts:?}");
        assert!(texts[1].contains("Try Porto.") && !texts.iter().any(|t| t.contains("Old answer")));
        assert_eq!(c.created_at.timestamp(), 1_700_000_000);
    }

    #[test]
    fn imports_claude_exports_and_zips() {
        let export = json!([{
            "uuid": "x", "name": "Rust question", "created_at": "2025-01-02T10:00:00Z", "updated_at": "2025-01-02T10:05:00Z",
            "chat_messages": [
                { "sender": "human", "text": "What is a lifetime?", "created_at": "2025-01-02T10:00:00Z" },
                { "sender": "assistant", "text": "", "content": [{ "type": "text", "text": "A scope for references." }] }
            ]
        }]);
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("export.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("data/conversations.json", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zip, export.to_string().as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let report = import(dir.path(), &zip_path).unwrap();
        assert_eq!(report, ImportReport { imported: 1, skipped: 0, source: "Claude".into() });
        let listed = store::list_conversations(dir.path());
        assert_eq!(listed[0].title, "Rust question");
        assert!(convert(&json!({"not": "a list"})).is_err());
        assert!(convert(&json!([{"foo": 1}])).is_err());
    }
}
