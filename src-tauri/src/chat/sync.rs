//! Encrypted sync between devices (Osaurus's): conversations, agents, knowledge, memories,
//! projects, canvases and skills kept the same on each computer through a folder that a sync
//! service mirrors (Syncthing, Nextcloud, Dropbox, iCloud Drive, OneDrive…).
//!
//! Nothing is written there in the clear. Each file becomes one item, `items/<id>.cortex`, sealed
//! with XChaCha20-Poly1305 under a key derived from the user's passphrase (Argon2id, with a salt
//! kept in the folder's `keycheck.json`, which also tells a wrong passphrase from a right one).
//! The item's name is a keyed hash of its path, so file names and folder layout stay hidden too.
//!
//! Each device remembers the content hash it last synced for every path (`chat/sync-state.json`,
//! never synced). A side that changed since then wins; when both changed, the newer edit wins
//! and the conflict is reported. Deleting leaves a tombstone, so a deletion reaches the other
//! devices instead of the file coming back; a deleted conversation goes to their trash.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The folder Cortex makes inside the chosen one.
pub const DIR_NAME: &str = "Cortex Sync";
const MAGIC: &[u8] = b"LUMOSYNC1";
const CHECK_TEXT: &[u8] = b"cortex-sync-check";
const PASSPHRASE: &str = "sync:passphrase";
/// Larger files are left out (attachments live inside conversations already).
const MAX_FILE: u64 = 25 * 1024 * 1024;

/// Device settings (not synced).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The folder the sync service mirrors; Cortex works in `<folder>/Cortex Sync`.
    pub folder: Option<String>,
    /// Minutes between automatic syncs; 0 syncs only when asked.
    pub every_minutes: u32,
    pub last_sync: Option<chrono::DateTime<chrono::Utc>>,
    pub last_report: String,
}

/// What one sync did.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub sent: usize,
    pub received: usize,
    pub deleted: usize,
    pub conflicts: Vec<String>,
    pub errors: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.sent > 0 {
            parts.push(format!("{} sent", self.sent));
        }
        if self.received > 0 {
            parts.push(format!("{} received", self.received));
        }
        if self.deleted > 0 {
            parts.push(format!("{} deleted", self.deleted));
        }
        if !self.conflicts.is_empty() {
            parts.push(format!(
                "{} conflict{} (newer kept)",
                self.conflicts.len(),
                if self.conflicts.len() == 1 { "" } else { "s" }
            ));
        }
        if !self.errors.is_empty() {
            parts.push(format!(
                "{} error{}",
                self.errors.len(),
                if self.errors.len() == 1 { "" } else { "s" }
            ));
        }
        if parts.is_empty() {
            "Everything was already in sync.".into()
        } else {
            format!("{}.", parts.join(", "))
        }
    }
}

/// One synced file, as sealed in its item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Item {
    path: String,
    hash: String,
    /// Milliseconds since the epoch, of the edit (or deletion) this item carries.
    modified: i64,
    device: String,
    deleted: bool,
    /// Base64 of the file; empty for a deletion.
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyCheck {
    version: u32,
    salt: String,
    check: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    /// Which store this state is for (its salt): another folder starts from nothing.
    #[serde(default)]
    store: String,
    /// The content hash last synced, by path.
    synced: BTreeMap<String, String>,
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat").join("sync.json")
}

fn state_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat").join("sync-state.json")
}

pub fn settings(config_dir: &Path) -> Settings {
    read(&settings_path(config_dir)).unwrap_or_default()
}

pub fn save_settings(config_dir: &Path, s: &Settings) -> Result<(), String> {
    write(&settings_path(config_dir), s)
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&crate::chat::at_rest::read_to_string(path).ok()?).ok()
}

fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    crate::chat::at_rest::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

// ---- keys ----------------------------------------------------------------------------------

/// The key from the passphrase and salt (Argon2id), kept for the session: deriving is slow on
/// purpose.
fn derive(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    /// The last key derived: (passphrase, salt, key).
    type Derived = (String, Vec<u8>, [u8; 32]);
    static CACHE: Mutex<Option<Derived>> = Mutex::new(None);
    let hashed = sha256(passphrase.as_bytes());
    if let Some((p, s, k)) = CACHE.lock().unwrap().as_ref() {
        if *p == hashed && s == salt {
            return Ok(*k);
        }
    }
    let mut key = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    *CACHE.lock().unwrap() = Some((hashed, salt.to_vec(), key));
    Ok(key)
}

fn seal(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; 24];
    rand::fill(&mut nonce);
    let sealed = cipher
        .encrypt(XNonce::from_slice(&nonce), plain)
        .map_err(|_| "could not encrypt".to_string())?;
    Ok([MAGIC, &nonce[..], &sealed].concat())
}

fn open(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    let rest = data.strip_prefix(MAGIC).ok_or("not a Cortex sync file")?;
    if rest.len() < 24 {
        return Err("a sync file is cut short".into());
    }
    let (nonce, sealed) = rest.split_at(24);
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), sealed)
        .map_err(|_| "wrong passphrase, or a damaged sync file".to_string())
}

/// The item's file name: a keyed hash of the path, so the folder shows no names.
fn item_name(key: &[u8; 32], path: &str) -> String {
    let mut h = Sha256::new();
    h.update(key);
    h.update(path.as_bytes());
    format!("{}.lumo", &hex(&h.finalize())[..40])
}

/// Opens the sync folder with `passphrase`: the first device sets it up, the others must give
/// the same passphrase.
pub fn unlock(remote: &Path, passphrase: &str) -> Result<[u8; 32], String> {
    if passphrase.chars().count() < 8 {
        return Err("Use a passphrase of at least 8 characters.".into());
    }
    let check_path = remote.join("keycheck.json");
    let b64 = base64::engine::general_purpose::STANDARD;
    if let Some(check) = read::<KeyCheck>(&check_path) {
        let salt = b64.decode(&check.salt).map_err(|e| e.to_string())?;
        let key = derive(passphrase, &salt)?;
        let sealed = b64.decode(&check.check).map_err(|e| e.to_string())?;
        return match open(&key, &sealed) {
            Ok(text) if text == CHECK_TEXT => Ok(key),
            _ => Err("That passphrase does not open this sync folder. Use the one set on your other device.".into()),
        };
    }
    std::fs::create_dir_all(remote.join("items"))
        .map_err(|e| format!("Could not create the sync folder: {e}"))?;
    let mut salt = [0u8; 16];
    rand::fill(&mut salt);
    let key = derive(passphrase, &salt)?;
    write(
        &check_path,
        &KeyCheck {
            version: 1,
            salt: b64.encode(salt),
            check: b64.encode(seal(&key, CHECK_TEXT)?),
        },
    )?;
    Ok(key)
}

// ---- what syncs ----------------------------------------------------------------------------

/// Paths (relative to the config folder, with `/`) of the files that sync.
fn local_files(config_dir: &Path) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let mut add = |rel: String, full: PathBuf| {
        if std::fs::metadata(&full).is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE) {
            out.insert(rel, full);
        }
    };
    for file in [
        "agents.json",
        "memories.json",
        "knowledge.json",
        "projects.json",
        "instructions.json",
    ] {
        add(format!("chat/{file}"), config_dir.join("chat").join(file));
    }
    if let Ok(entries) = std::fs::read_dir(config_dir.join("chat").join("conversations")) {
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            // Temporary chats are never kept, so they never sync.
            let temporary = crate::chat::at_rest::read_to_string(e.path())
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .is_some_and(|v| v.get("temporary").and_then(|t| t.as_bool()) == Some(true));
            if name.ends_with(".json") && !temporary {
                add(format!("chat/conversations/{name}"), e.path());
            }
        }
    }
    for dir in ["chat/agents", "chat/canvases", "skills"] {
        walk(&config_dir.join(dir), dir, &mut add);
    }
    out
}

fn walk(dir: &Path, rel: &str, add: &mut impl FnMut(String, PathBuf)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.ends_with(".tmp") {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            walk(&path, &format!("{rel}/{name}"), add);
        } else {
            add(format!("{rel}/{name}"), path);
        }
    }
}

/// Whether a path from the sync folder may be written here: only the kinds of files that sync,
/// and never outside the config folder.
fn allowed(path: &str) -> bool {
    let safe = !path
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == ".." || p.contains('\\'))
        && !path.contains(':');
    safe && (matches!(
        path,
        "chat/agents.json"
            | "chat/memories.json"
            | "chat/knowledge.json"
            | "chat/projects.json"
            | "chat/instructions.json"
    ) || (path.starts_with("chat/conversations/")
        && path.ends_with(".json")
        && path.matches('/').count() == 2)
        || path.starts_with("chat/agents/")
        || path.starts_with("chat/canvases/")
        || path.starts_with("skills/"))
}

// ---- the sync itself -----------------------------------------------------------------------

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).timestamp_millis())
        .unwrap_or_else(now_ms)
}

fn device_name() -> String {
    hostname().unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| crate::chat::at_rest::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Syncs the config folder with `<folder>/Cortex Sync` under `key`.
pub fn sync_once(config_dir: &Path, remote: &Path, key: &[u8; 32]) -> Report {
    let mut report = Report::default();
    let b64 = base64::engine::general_purpose::STANDARD;
    let items_dir = remote.join("items");
    if let Err(e) = std::fs::create_dir_all(&items_dir) {
        report.errors.push(format!("the sync folder: {e}"));
        return report;
    }
    let store = read::<KeyCheck>(&remote.join("keycheck.json"))
        .map(|c| c.salt)
        .unwrap_or_default();
    let mut state: State = read(&state_path(config_dir))
        .filter(|s: &State| s.store == store)
        .unwrap_or_default();
    state.store = store;

    // What the folder holds, by path.
    let mut remote_items: BTreeMap<String, Item> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&items_dir) {
        for e in entries.filter_map(Result::ok) {
            if !e.file_name().to_string_lossy().ends_with(".lumo") {
                continue;
            }
            let item = crate::chat::at_rest::read(e.path())
                .map_err(|e| e.to_string())
                .and_then(|d| open(key, &d))
                .and_then(|p| serde_json::from_slice::<Item>(&p).map_err(|e| e.to_string()));
            match item {
                // A name that does not match its path was not written by Cortex with this key.
                Ok(item)
                    if item_name(key, &item.path) == e.file_name().to_string_lossy()
                        && allowed(&item.path) =>
                {
                    remote_items.insert(item.path.clone(), item);
                }
                Ok(item) => report
                    .errors
                    .push(format!("{} is not where it belongs; left alone", item.path)),
                Err(err) => report
                    .errors
                    .push(format!("an item could not be read ({err})")),
            }
        }
    }

    let local = local_files(config_dir);
    let mut local_hashes: BTreeMap<String, (String, Vec<u8>)> = BTreeMap::new();
    for (rel, full) in &local {
        match crate::chat::at_rest::read(full) {
            Ok(bytes) => {
                local_hashes.insert(rel.clone(), (sha256(&bytes), bytes));
            }
            Err(e) => report.errors.push(format!("{rel}: {e}")),
        }
    }

    let device = device_name();
    let put = |item: &Item| -> Result<(), String> {
        let sealed = seal(key, &serde_json::to_vec(item).map_err(|e| e.to_string())?)?;
        let path = items_dir.join(item_name(key, &item.path));
        let tmp = path.with_extension("part");
        crate::chat::at_rest::write(&tmp, sealed).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    };
    let upload = |rel: &str, hash: &str, bytes: &[u8]| {
        put(&Item {
            path: rel.into(),
            hash: hash.into(),
            modified: modified_ms(&local[rel]),
            device: device.clone(),
            deleted: false,
            content: b64.encode(bytes),
        })
    };
    let download = |item: &Item| -> Result<(), String> {
        let bytes = b64.decode(&item.content).map_err(|e| e.to_string())?;
        if sha256(&bytes) != item.hash {
            return Err("its contents do not match its hash".into());
        }
        let target = config_dir.join(&item.path);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        // An overwritten conversation keeps a backup, as any other change does.
        if item.path.starts_with("chat/conversations/") {
            super::backup::before_overwrite(config_dir, &target);
        }
        let tmp = target.with_extension("sync-tmp");
        crate::chat::at_rest::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
    };
    let remove_local = |rel: &str| -> Result<(), String> {
        if let Some(id) = rel
            .strip_prefix("chat/conversations/")
            .and_then(|n| n.strip_suffix(".json"))
        {
            return super::store::delete_conversation(config_dir, id);
        }
        std::fs::remove_file(config_dir.join(rel)).map_err(|e| e.to_string())
    };

    let paths: BTreeSet<String> = local_hashes
        .keys()
        .chain(remote_items.keys())
        .chain(state.synced.keys())
        .cloned()
        .collect();
    for path in paths {
        let l = local_hashes.get(&path);
        let r = remote_items.get(&path);
        let s = state.synced.get(&path).cloned();
        let result: Result<Option<String>, String> = match (l, r) {
            (Some((lh, bytes)), None) => upload(&path, lh, bytes).map(|_| {
                report.sent += 1;
                Some(lh.clone())
            }),
            (None, Some(item)) if !item.deleted => {
                if s.as_deref() == Some(item.hash.as_str()) {
                    // Synced before and gone here: deleted on this device.
                    put(&Item {
                        path: path.clone(),
                        hash: String::new(),
                        modified: now_ms(),
                        device: device.clone(),
                        deleted: true,
                        content: String::new(),
                    })
                    .map(|_| {
                        report.deleted += 1;
                        None
                    })
                } else {
                    download(item).map(|_| {
                        report.received += 1;
                        Some(item.hash.clone())
                    })
                }
            }
            (None, Some(_)) => Ok(None),
            (Some((lh, bytes)), Some(item)) if item.deleted => {
                if s.as_deref() == Some(lh.as_str()) {
                    remove_local(&path).map(|_| {
                        report.deleted += 1;
                        None
                    })
                } else {
                    // Changed here after it was deleted elsewhere: it comes back.
                    upload(&path, lh, bytes).map(|_| {
                        report.sent += 1;
                        Some(lh.clone())
                    })
                }
            }
            (Some((lh, bytes)), Some(item)) => {
                if *lh == item.hash {
                    Ok(Some(lh.clone()))
                } else if s.as_deref() == Some(lh.as_str()) {
                    download(item).map(|_| {
                        report.received += 1;
                        Some(item.hash.clone())
                    })
                } else if s.as_deref() == Some(item.hash.as_str()) {
                    upload(&path, lh, bytes).map(|_| {
                        report.sent += 1;
                        Some(lh.clone())
                    })
                } else {
                    report.conflicts.push(path.clone());
                    if item.modified > modified_ms(&local[&path]) {
                        download(item).map(|_| {
                            report.received += 1;
                            Some(item.hash.clone())
                        })
                    } else {
                        upload(&path, lh, bytes).map(|_| {
                            report.sent += 1;
                            Some(lh.clone())
                        })
                    }
                }
            }
            (None, None) => Ok(None),
        };
        match result {
            Ok(Some(hash)) => {
                state.synced.insert(path, hash);
            }
            Ok(None) => {
                state.synced.remove(&path);
            }
            Err(e) => report.errors.push(format!("{path}: {e}")),
        }
    }
    if let Err(e) = write(&state_path(config_dir), &state) {
        report.errors.push(format!("the sync state: {e}"));
    }
    report
}

// ---- passphrase and running ----------------------------------------------------------------

pub async fn passphrase() -> Result<Option<String>, String> {
    crate::chat::secrets::local()?.load(PASSPHRASE).await
}

pub async fn set_passphrase(value: Option<&str>) -> Result<(), String> {
    let store = crate::chat::secrets::local()?;
    match value {
        Some(v) => store.store(PASSPHRASE, v).await,
        None => store.delete(PASSPHRASE).await,
    }
}

/// Where Cortex works for a chosen folder: the folder's own `Cortex Sync`, or, when the choice is
/// a `Cortex Sync` folder or inside one (its `items`), that folder, so picking a folder one level
/// too deep never starts a second, separate store.
pub fn remote_dir(folder: &str) -> PathBuf {
    let folder = PathBuf::from(folder);
    folder
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == DIR_NAME))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| folder.join(DIR_NAME))
}

/// One sync with the saved settings and passphrase; the report is also saved.
pub async fn run(config_dir: &Path) -> Result<Report, String> {
    static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = RUNNING.lock().await;
    let mut s = settings(config_dir);
    let folder = s.folder.clone().ok_or("Choose a sync folder first.")?;
    let phrase = passphrase()
        .await?
        .ok_or("Set the sync passphrase first.")?;
    let dir = config_dir.to_path_buf();
    let report = tokio::task::spawn_blocking(move || {
        let remote = remote_dir(&folder);
        unlock(&remote, &phrase).map(|key| sync_once(&dir, &remote, &key))
    })
    .await
    .map_err(|e| e.to_string())??;
    s.last_sync = Some(chrono::Utc::now());
    s.last_report = report.summary();
    save_settings(config_dir, &s)?;
    info!("sync: {}", s.last_report);
    for e in &report.errors {
        warn!("sync: {e}");
    }
    Ok(report)
}

/// Syncs in the background every few minutes, as set; tells the window when something arrived.
pub fn start(app: tauri::AppHandle) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let Ok(config_dir) = crate::settings::get_config_dir(&app) else {
            return;
        };
        // A first sync shortly after start, then on the chosen interval.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            let every = settings(&config_dir).every_minutes;
            if every > 0 && settings(&config_dir).folder.is_some() {
                match run(&config_dir).await {
                    Ok(report) if report.received > 0 || report.deleted > 0 => {
                        let _ = app.emit("chat://synced", report.summary());
                    }
                    Ok(_) => {}
                    Err(e) => warn!("sync: {e}"),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60 * u64::from(every.max(1)))).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            crate::chat::at_rest::write(p, text).unwrap();
        }
        dir
    }

    fn text(dir: &Path, rel: &str) -> Option<String> {
        crate::chat::at_rest::read_to_string(dir.join(rel)).ok()
    }

    #[test]
    fn sealing_round_trips_and_needs_the_key() {
        let key = derive("correct horse battery", b"0123456789abcdef").unwrap();
        let sealed = seal(&key, b"hello").unwrap();
        assert!(sealed.starts_with(MAGIC) && !sealed.windows(5).any(|w| w == b"hello"));
        assert_eq!(open(&key, &sealed).unwrap(), b"hello");
        let other = derive("another passphrase", b"0123456789abcdef").unwrap();
        assert!(open(&other, &sealed).is_err());
    }

    #[test]
    fn the_folder_checks_the_passphrase() {
        let folder = tempfile::tempdir().unwrap();
        let remote = folder.path().join(DIR_NAME);
        assert!(unlock(&remote, "short").is_err());
        let key = unlock(&remote, "first passphrase").unwrap();
        assert_eq!(unlock(&remote, "first passphrase").unwrap(), key);
        assert!(unlock(&remote, "wrong passphrase")
            .unwrap_err()
            .contains("does not open"));
    }

    #[test]
    fn two_devices_converge_and_deletions_travel() {
        let folder = tempfile::tempdir().unwrap();
        let remote = folder.path().join(DIR_NAME);
        let key = unlock(&remote, "shared passphrase").unwrap();
        let a = device(&[
            (
                "chat/conversations/c1.json",
                r#"{"id":"c1","title":"From A"}"#,
            ),
            ("chat/memories.json", "[]"),
            ("skills/s/SKILL.md", "# Skill"),
            ("chat/providers.json", "{\"secret\":1}"),
        ]);
        let b = device(&[
            (
                "chat/conversations/c2.json",
                r#"{"id":"c2","title":"From B"}"#,
            ),
            (
                "chat/conversations/t.json",
                r#"{"id":"t","temporary":true}"#,
            ),
        ]);

        let r = sync_once(a.path(), &remote, &key);
        assert_eq!((r.sent, r.received), (3, 0), "{r:?}");
        // Nothing readable lands in the folder: no names, no text.
        for e in std::fs::read_dir(remote.join("items")).unwrap() {
            let bytes = crate::chat::at_rest::read(e.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("From A"));
        }
        let r = sync_once(b.path(), &remote, &key);
        assert_eq!((r.sent, r.received), (1, 3), "{r:?}");
        assert_eq!(
            text(b.path(), "chat/conversations/c1.json").unwrap(),
            r#"{"id":"c1","title":"From A"}"#
        );
        assert_eq!(text(b.path(), "skills/s/SKILL.md").unwrap(), "# Skill");
        assert!(
            text(b.path(), "chat/providers.json").is_none(),
            "device settings and keys stay home"
        );
        sync_once(a.path(), &remote, &key);
        assert!(text(a.path(), "chat/conversations/c2.json").is_some());
        assert!(
            text(a.path(), "chat/conversations/t.json").is_none(),
            "temporary chats stay home"
        );

        // An edit on B reaches A; nothing changes when both are in step.
        crate::chat::at_rest::write(b.path().join("chat/memories.json"), "[\"likes tea\"]")
            .unwrap();
        assert_eq!(sync_once(b.path(), &remote, &key).sent, 1);
        assert_eq!(sync_once(a.path(), &remote, &key).received, 1);
        assert_eq!(
            text(a.path(), "chat/memories.json").unwrap(),
            "[\"likes tea\"]"
        );
        assert_eq!(
            sync_once(a.path(), &remote, &key).summary(),
            "Everything was already in sync."
        );

        // A deletion on A reaches B instead of the file coming back.
        std::fs::remove_file(a.path().join("skills/s/SKILL.md")).unwrap();
        assert_eq!(sync_once(a.path(), &remote, &key).deleted, 1);
        assert_eq!(sync_once(b.path(), &remote, &key).deleted, 1);
        assert!(text(b.path(), "skills/s/SKILL.md").is_none());
        assert_eq!(
            sync_once(a.path(), &remote, &key).summary(),
            "Everything was already in sync."
        );

        // Both edit the same file: one conflict, the newer edit on both.
        crate::chat::at_rest::write(a.path().join("chat/memories.json"), "[\"A\"]").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::chat::at_rest::write(b.path().join("chat/memories.json"), "[\"B newer\"]").unwrap();
        sync_once(a.path(), &remote, &key);
        let r = sync_once(b.path(), &remote, &key);
        assert_eq!(r.conflicts, ["chat/memories.json"]);
        sync_once(a.path(), &remote, &key);
        assert_eq!(
            text(a.path(), "chat/memories.json").unwrap(),
            "[\"B newer\"]"
        );
    }

    #[test]
    fn what_was_synced_with_one_store_does_not_count_for_another() {
        let a = device(&[("chat/memories.json", "[\"edited here\"]")]);
        let first = tempfile::tempdir().unwrap();
        let k1 = unlock(&first.path().join(DIR_NAME), "first store pass").unwrap();
        sync_once(a.path(), &first.path().join(DIR_NAME), &k1);
        // Another store already holds an older version from another device.
        let second = tempfile::tempdir().unwrap();
        let remote = second.path().join(DIR_NAME);
        let k2 = unlock(&remote, "second store pass").unwrap();
        let b = device(&[("chat/memories.json", "[\"old\"]")]);
        sync_once(b.path(), &remote, &k2);
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::chat::at_rest::write(a.path().join("chat/memories.json"), "[\"edited here\"]")
            .unwrap();
        let r = sync_once(a.path(), &remote, &k2);
        assert_eq!(
            r.conflicts,
            ["chat/memories.json"],
            "a first sync with a store treats differences as conflicts"
        );
        assert_eq!(
            text(a.path(), "chat/memories.json").unwrap(),
            "[\"edited here\"]",
            "the newer local edit is kept"
        );
    }

    #[test]
    fn a_folder_inside_the_store_means_the_store() {
        assert_eq!(
            remote_dir("/x/Dropbox"),
            PathBuf::from("/x/Dropbox/Cortex Sync")
        );
        assert_eq!(
            remote_dir("/x/Dropbox/Cortex Sync"),
            PathBuf::from("/x/Dropbox/Cortex Sync")
        );
        assert_eq!(
            remote_dir("/x/Dropbox/Cortex Sync/items"),
            PathBuf::from("/x/Dropbox/Cortex Sync")
        );
    }

    #[test]
    fn only_known_paths_inside_the_config_folder_are_written() {
        assert!(
            allowed("chat/conversations/abc.json")
                && allowed("skills/x/SKILL.md")
                && allowed("chat/agents.json")
        );
        for bad in [
            "../x",
            "chat/conversations/../../x.json",
            "chat/providers.json",
            "/etc/passwd",
            "chat/conversations/a/b.json",
            "skills//x",
            "C:/x",
            "chat\\x",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
    }
}
