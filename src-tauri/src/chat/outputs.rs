//! What chats made (Onyx's Craft outputs, LobeHub's resource library): canvases, apps and
//! sheets, and the files left in a chat's workspace by code it ran or images it generated.
//! One chat's outputs fill its Outputs panel; everyone's fill the Resources page.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub conversation_id: String,
    /// `document`, `app`, `sheet`, `image` or `file`.
    pub kind: String,
    pub title: String,
    /// The canvas id, or the file's path inside the workspace.
    pub id: String,
    pub bytes: u64,
    pub at: Option<DateTime<Utc>>,
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "svg"];
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov"];
/// Files listed per workspace.
const MAX_FILES: usize = 300;

fn walk(base: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, std::fs::Metadata)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        // Hidden files, caches and environments are not outputs.
        if name.starts_with('.') || matches!(name.as_str(), "__pycache__" | "node_modules" | "venv" | ".venv") {
            continue;
        }
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            if depth < 4 {
                walk(base, &path, depth + 1, out);
            }
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), meta));
        }
    }
}

/// The files in a chat's workspace.
pub fn files(config_dir: &Path, conversation_id: &str) -> Vec<Output> {
    let Ok(dir) = super::sandbox::workspace(config_dir, conversation_id) else { return Vec::new() };
    let mut found = Vec::new();
    walk(&dir, &dir, 0, &mut found);
    found
        .into_iter()
        .map(|(rel, meta)| {
            let ext = rel.rsplit('.').next().unwrap_or_default().to_lowercase();
            Output {
                conversation_id: conversation_id.to_string(),
                kind: if IMAGE_EXTS.contains(&ext.as_str()) { "image" } else if VIDEO_EXTS.contains(&ext.as_str()) { "video" } else { "file" }.into(),
                title: rel.rsplit('/').next().unwrap_or(&rel).to_string(),
                id: rel,
                bytes: meta.len(),
                at: meta.modified().ok().map(DateTime::<Utc>::from),
            }
        })
        .collect()
}

fn canvases(list: Vec<super::canvas::Canvas>) -> Vec<Output> {
    list.into_iter()
        .map(|c| Output {
            bytes: c.versions.last().map_or(0, |v| v.content.len() as u64),
            at: c.versions.last().map(|v| v.at),
            conversation_id: c.conversation_id,
            kind: c.kind,
            title: c.title,
            id: c.id,
        })
        .collect()
}

fn newest_first(mut out: Vec<Output>) -> Vec<Output> {
    out.sort_by_key(|a| std::cmp::Reverse(a.at));
    out
}

/// What one chat made.
pub fn for_conversation(config_dir: &Path, conversation_id: &str) -> Vec<Output> {
    let mut out = canvases(super::canvas::list_for(config_dir, conversation_id));
    out.extend(files(config_dir, conversation_id));
    newest_first(out)
}

/// What every chat made.
pub fn all(config_dir: &Path) -> Vec<Output> {
    let mut out = canvases(super::canvas::list_all(config_dir));
    let sandbox = super::store::chat_dir(config_dir).join("sandbox");
    for entry in std::fs::read_dir(sandbox).into_iter().flatten().flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if super::store::valid_id(&id) {
            out.extend(files(config_dir, &id));
        }
    }
    newest_first(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_gather_canvases_and_workspace_files() {
        let dir = tempfile::tempdir().unwrap();
        let ws = super::super::sandbox::workspace(dir.path(), "c1").unwrap();
        std::fs::create_dir_all(ws.join("images")).unwrap();
        std::fs::create_dir_all(ws.join("__pycache__")).unwrap();
        std::fs::write(ws.join("images/cat.png"), b"png").unwrap();
        std::fs::write(ws.join("report.csv"), b"a,b").unwrap();
        std::fs::write(ws.join(".hidden"), b"x").unwrap();
        std::fs::write(ws.join("__pycache__/x.pyc"), b"x").unwrap();
        super::super::canvas::write(dir.path(), "c1", None, Some("Plan"), "# Plan", "assistant").unwrap();
        let mine = for_conversation(dir.path(), "c1");
        let mut kinds: Vec<(&str, &str)> = mine.iter().map(|o| (o.kind.as_str(), o.title.as_str())).collect();
        kinds.sort();
        assert_eq!(kinds, [("document", "Plan"), ("file", "report.csv"), ("image", "cat.png")]);
        assert_eq!(mine.iter().find(|o| o.kind == "image").unwrap().id, "images/cat.png");
        assert_eq!(all(dir.path()).len(), 3);
        assert!(for_conversation(dir.path(), "other").is_empty());
    }
}
