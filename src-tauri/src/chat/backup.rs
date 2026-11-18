//! Keeping conversations safe over years: daily backups, a trash, and recovery.
//!
//! - **Backups.** The first time a conversation changes on a given day, the file as it was is
//!   copied to `chat/backups/<date>/conversations/`. The small state files (projects, knowledge,
//!   tasks, agents…) are copied once a day. Backups older than [`KEEP_DAYS`] are removed.
//! - **Trash.** Deleting a conversation moves it to `chat/trash/`, where it stays for
//!   [`KEEP_DAYS`] and can be restored.
//! - **Recovery.** A conversation file that cannot be read is replaced by its newest backup; the
//!   damaged file is kept in `chat/quarantine/`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::Serialize;

use super::store::{chat_dir, valid_id, Conversation};

/// Days a backup or a deleted conversation is kept.
pub const KEEP_DAYS: i64 = 30;

fn backups_dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("backups")
}

fn trash_dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("trash")
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn conversation_file(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("invalid conversation id `{id}`"));
    }
    Ok(chat_dir(config_dir).join("conversations").join(format!("{id}.json")))
}

fn readable(path: &Path) -> Option<Conversation> {
    serde_json::from_str(&crate::chat::at_rest::read_to_string(path).ok()?).ok()
}

/// Called before a conversation file is replaced: keeps today's first copy of it.
pub fn before_overwrite(config_dir: &Path, path: &Path) {
    let Some(name) = path.file_name() else { return };
    if !path.exists() {
        return;
    }
    let dest = backups_dir(config_dir).join(today()).join("conversations").join(name);
    if dest.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::copy(path, &dest) {
        log::warn!("could not back up {path:?}: {e}");
    }
}

/// Once a day: copies the state files and removes backups and trash past [`KEEP_DAYS`].
pub fn daily(config_dir: &Path) {
    let chat = chat_dir(config_dir);
    let state = backups_dir(config_dir).join(today()).join("state");
    if !state.exists() && std::fs::create_dir_all(&state).is_ok() {
        for entry in std::fs::read_dir(&chat).into_iter().flatten().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|x| x == "json") {
                let _ = std::fs::copy(&path, state.join(entry.file_name()));
            }
        }
    }
    let cutoff = Local::now().date_naive() - chrono::Duration::days(KEEP_DAYS);
    for entry in std::fs::read_dir(backups_dir(config_dir)).into_iter().flatten().filter_map(Result::ok) {
        let old = entry.file_name().to_str().and_then(|n| NaiveDate::parse_from_str(n, "%Y-%m-%d").ok()).is_some_and(|d| d < cutoff);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let expired = SystemTime::now() - Duration::from_secs(KEEP_DAYS as u64 * 86_400);
    for entry in std::fs::read_dir(trash_dir(config_dir)).into_iter().flatten().filter_map(Result::ok) {
        if entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < expired) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Moves a conversation to the trash. A temporary one is deleted outright.
pub fn trash(config_dir: &Path, id: &str) -> Result<(), String> {
    let path = conversation_file(config_dir, id)?;
    if !path.exists() {
        return Ok(());
    }
    if readable(&path).is_some_and(|c| c.temporary) {
        return std::fs::remove_file(&path).map_err(|e| e.to_string());
    }
    let dir = trash_dir(config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(format!("{id}.json"));
    std::fs::rename(&path, &dest).map_err(|e| e.to_string())?;
    // The file's time becomes when it was deleted, which is what the 30 days count from.
    if let Ok(file) = std::fs::File::options().write(true).open(&dest) {
        let _ = file.set_modified(SystemTime::now());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    pub id: String,
    pub title: String,
    pub deleted_at: DateTime<Utc>,
    pub days_left: i64,
}

/// Deleted conversations, newest first.
pub fn list_trash(config_dir: &Path) -> Vec<TrashItem> {
    let mut out: Vec<TrashItem> = std::fs::read_dir(trash_dir(config_dir))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_str()?.to_string();
            let deleted_at: DateTime<Utc> = entry.metadata().ok()?.modified().ok()?.into();
            let title = readable(&entry.path()).map(|c| c.title).filter(|t| !t.is_empty()).unwrap_or_else(|| "Untitled".into());
            let days_left = (KEEP_DAYS - (Utc::now() - deleted_at).num_days()).max(0);
            Some(TrashItem { id, title, deleted_at, days_left })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.deleted_at));
    out
}

/// Puts a deleted conversation back.
pub fn restore(config_dir: &Path, id: &str) -> Result<(), String> {
    let dest = conversation_file(config_dir, id)?;
    let from = trash_dir(config_dir).join(format!("{id}.json"));
    if dest.exists() {
        return Err("A conversation with that id already exists.".into());
    }
    std::fs::rename(&from, &dest).map_err(|_| "That conversation is no longer in the trash.".to_string())
}

/// Deletes one conversation from the trash for good, or all of them.
pub fn delete_forever(config_dir: &Path, id: Option<&str>) -> Result<(), String> {
    match id {
        Some(id) => {
            conversation_file(config_dir, id)?;
            std::fs::remove_file(trash_dir(config_dir).join(format!("{id}.json"))).map_err(|e| e.to_string())
        }
        None => match std::fs::remove_dir_all(trash_dir(config_dir)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    }
}

/// The newest readable backup of a conversation: its date and file.
pub fn latest_backup(config_dir: &Path, id: &str) -> Option<(String, PathBuf)> {
    let mut days: Vec<String> = std::fs::read_dir(backups_dir(config_dir)).ok()?.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect();
    days.sort();
    days.into_iter().rev().find_map(|day| {
        let path = backups_dir(config_dir).join(&day).join("conversations").join(format!("{id}.json"));
        readable(&path).map(|_| (day, path))
    })
}

/// Replaces a damaged conversation with its newest backup; returns the backup's date.
pub fn recover(config_dir: &Path, id: &str) -> Result<String, String> {
    let path = conversation_file(config_dir, id)?;
    let (day, backup) = latest_backup(config_dir, id).ok_or("There is no backup of that conversation.")?;
    if path.exists() {
        let quarantine = chat_dir(config_dir).join("quarantine");
        std::fs::create_dir_all(&quarantine).map_err(|e| e.to_string())?;
        std::fs::rename(&path, quarantine.join(format!("{id}-{}.json", Utc::now().format("%Y%m%d%H%M%S")))).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&backup, &path).map_err(|e| e.to_string())?;
    Ok(day)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Damaged {
    pub id: String,
    /// The date of the newest backup that can replace it.
    pub backup: Option<String>,
}

/// How the stored data is doing, for Settings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub conversations: usize,
    pub damaged: Vec<Damaged>,
    pub trash: usize,
    /// Days that have a backup.
    pub backup_days: usize,
    pub newest_backup: Option<String>,
    /// Bytes under `chat/`, backups included.
    pub bytes: u64,
    pub keep_days: i64,
}

pub fn health(config_dir: &Path) -> Health {
    let mut days: Vec<String> = std::fs::read_dir(backups_dir(config_dir)).into_iter().flatten().filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect();
    days.sort();
    let bytes = walkdir::WalkDir::new(chat_dir(config_dir)).into_iter().filter_map(Result::ok).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum();
    Health {
        conversations: super::index::list(config_dir).len(),
        damaged: super::index::damaged(config_dir).into_iter().map(|id| Damaged { backup: latest_backup(config_dir, &id).map(|(day, _)| day), id }).collect(),
        trash: list_trash(config_dir).len(),
        backup_days: days.len(),
        newest_backup: days.last().cloned(),
        bytes,
        keep_days: KEEP_DAYS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::store::{delete_conversation, load_conversation, save_conversation};

    const ID: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn backs_up_trashes_restores_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Conversation::new(ID.into(), None, None);
        c.title = "First".into();
        save_conversation(dir.path(), &c).unwrap();
        // Nothing to back up on the first save; the second keeps the earlier version.
        assert!(latest_backup(dir.path(), ID).is_none());
        c.title = "Second".into();
        save_conversation(dir.path(), &c).unwrap();
        c.title = "Third".into();
        save_conversation(dir.path(), &c).unwrap();
        let (day, backup) = latest_backup(dir.path(), ID).unwrap();
        assert_eq!(day, today());
        assert_eq!(readable(&backup).unwrap().title, "First");

        // Deleting moves it to the trash; restoring brings it back.
        delete_conversation(dir.path(), ID).unwrap();
        assert!(load_conversation(dir.path(), ID).is_err());
        let trash = list_trash(dir.path());
        assert_eq!((trash.len(), trash[0].title.as_str(), trash[0].days_left), (1, "Third", KEEP_DAYS));
        restore(dir.path(), ID).unwrap();
        assert_eq!(load_conversation(dir.path(), ID).unwrap().title, "Third");
        assert!(list_trash(dir.path()).is_empty() && restore(dir.path(), ID).is_err());

        // A damaged file is replaced by the backup and kept in quarantine.
        crate::chat::at_rest::write(conversation_file(dir.path(), ID).unwrap(), "{ broken").unwrap();
        let health = health(dir.path());
        assert_eq!(health.damaged.len(), 1);
        assert_eq!(health.damaged[0].backup.as_deref(), Some(today().as_str()));
        assert_eq!(recover(dir.path(), ID).unwrap(), today());
        assert_eq!(load_conversation(dir.path(), ID).unwrap().title, "First");
        assert_eq!(std::fs::read_dir(chat_dir(dir.path()).join("quarantine")).unwrap().count(), 1);

        // A temporary conversation is deleted outright.
        let mut temp = Conversation::new("22222222-2222-4222-8222-222222222222".into(), None, None);
        temp.temporary = true;
        save_conversation(dir.path(), &temp).unwrap();
        delete_conversation(dir.path(), &temp.id).unwrap();
        assert!(list_trash(dir.path()).is_empty());

        // State files are copied once a day; old backups and trash go.
        crate::chat::at_rest::write(chat_dir(dir.path()).join("projects.json"), "[]").unwrap();
        std::fs::create_dir_all(backups_dir(dir.path()).join("2020-01-01/conversations")).unwrap();
        daily(dir.path());
        assert!(backups_dir(dir.path()).join(today()).join("state/projects.json").exists());
        assert!(!backups_dir(dir.path()).join("2020-01-01").exists());
        delete_forever(dir.path(), None).unwrap();
    }
}
