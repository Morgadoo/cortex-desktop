//! Files attached lately, to attach again without finding them on disk (LibreChat's "recent
//! files"). What is kept is what the message carried: the text read from a document, or an image,
//! in `chat/recent-files.json`, newest first.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, write_json};

/// Files kept.
const KEEP: usize = 20;
/// Bytes of text or image data kept per file; larger files are not kept.
const MAX_FILE: usize = 2 * 1024 * 1024;
/// Bytes kept for all files together; the oldest go first.
const MAX_TOTAL: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentImage {
    pub media_type: String,
    /// Base64.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub name: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<RecentImage>,
    pub used_at: DateTime<Utc>,
}

impl RecentFile {
    fn size(&self) -> usize {
        self.text.len() + self.image.as_ref().map_or(0, |i| i.data.len())
    }

    fn same(&self, other: &RecentFile) -> bool {
        self.name == other.name && self.text == other.text && self.image == other.image
    }
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("recent-files.json")
}

pub fn load(config_dir: &Path) -> Vec<RecentFile> {
    std::fs::read_to_string(path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Puts the files first in the list (moving any the list already has), within the limits.
pub fn remember(list: &mut Vec<RecentFile>, files: Vec<RecentFile>) {
    for file in files.into_iter().rev() {
        if file.name.trim().is_empty() || file.size() == 0 || file.size() > MAX_FILE {
            continue;
        }
        list.retain(|f| !f.same(&file));
        list.insert(0, file);
    }
    list.truncate(KEEP);
    let mut total = 0;
    list.retain(|f| {
        total += f.size();
        total <= MAX_TOTAL
    });
}

pub fn add(config_dir: &Path, files: Vec<RecentFile>) -> Result<(), String> {
    let mut list = load(config_dir);
    remember(&mut list, files);
    write_json(&path(config_dir), &list)
}

/// Takes one file off the list, or every file when `index` is `None`.
pub fn forget(config_dir: &Path, index: Option<usize>) -> Result<Vec<RecentFile>, String> {
    let mut list = load(config_dir);
    match index {
        Some(i) if i < list.len() => {
            list.remove(i);
        }
        Some(_) => return Err("There is no such file.".into()),
        None => list.clear(),
    }
    write_json(&path(config_dir), &list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> RecentFile {
        RecentFile { name: name.into(), text: text.into(), image: None, used_at: Utc::now() }
    }

    #[test]
    fn newest_first_without_repeats_within_limits() {
        let mut list = Vec::new();
        remember(&mut list, vec![file("a.txt", "A"), file("b.txt", "B")]);
        assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["a.txt", "b.txt"]);
        // Attaching a file again moves it to the front instead of adding a copy.
        remember(&mut list, vec![file("b.txt", "B")]);
        assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["b.txt", "a.txt"]);
        // A changed file of the same name is another entry.
        remember(&mut list, vec![file("b.txt", "B2")]);
        assert_eq!(list.len(), 3);
        // Empty and very large files are not kept.
        remember(&mut list, vec![file("empty.txt", ""), file("big.txt", &"x".repeat(MAX_FILE + 1))]);
        assert_eq!(list.len(), 3);
        for i in 0..30 {
            remember(&mut list, vec![file(&format!("{i}.txt"), "x")]);
        }
        assert_eq!(list.len(), KEEP);
        assert_eq!(list[0].name, "29.txt");
        // The total size is bounded: the oldest go first.
        let mut list = Vec::new();
        for i in 0..8 {
            remember(&mut list, vec![file(&format!("{i}"), &"y".repeat(MAX_FILE))]);
        }
        assert_eq!(list.len(), MAX_TOTAL / MAX_FILE);
        assert_eq!(list[0].name, "7");
    }

    #[test]
    fn forget_one_or_all() {
        let dir = std::env::temp_dir().join(format!("lumo-recent-{}", std::process::id()));
        add(&dir, vec![file("a", "1"), file("b", "2")]).unwrap();
        assert_eq!(forget(&dir, Some(0)).unwrap().len(), 1);
        assert!(forget(&dir, Some(5)).is_err());
        assert!(forget(&dir, None).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
