//! Encryption at rest: when turned on (Settings → Profile → Encrypt data on this computer), every
//! file Cortex writes in its settings folder (conversations, the search index, knowledge, memories,
//! agents, tasks, canvases, libraries, backups) is sealed with XChaCha20-Poly1305 under a random
//! key kept in the OS keychain. Someone who copies the folder, or reads a backup of the disk,
//! sees only ciphertext.
//!
//! All of the chat code reads and writes through [`read`], [`read_to_string`] and [`write`]:
//! a sealed file (it starts with [`MAGIC`]) is opened with the key, a plain one is returned as it
//! is, so a half-converted folder still works. Folders that other programs read are left plain:
//! code sandboxes and agent-run folders (Python, Node and agent CLIs open those files), and
//! skills (their scripts run from disk). Files outside the settings folder are never sealed.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8] = b"LUMOREST1\0";
const KEY_NAME: &str = "at-rest-key";
/// Under the settings folder, never sealed.
const PLAIN: &[&str] = &[
    "chat/sandbox",
    "chat/agent-runs",
    "chat/encryption.json",
    "skills",
];

/// The settings folder and its key, while encryption is on and unlocked.
static STATE: RwLock<Option<(PathBuf, [u8; 32])>> = RwLock::new(None);
/// The settings folder, while it is encrypted but its key could not be read: nothing may be
/// written there, or plain files and empty defaults would replace sealed data.
static LOCKED: RwLock<Option<PathBuf>> = RwLock::new(None);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Marker {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    /// On, but the key could not be read from the keychain.
    pub locked: bool,
    pub sealed: usize,
    pub plain: usize,
}

fn marker_path(config_dir: &Path) -> PathBuf {
    config_dir.join("chat").join("encryption.json")
}

pub fn marker(config_dir: &Path) -> Marker {
    std::fs::read_to_string(marker_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Whether a file at `path` is sealed when written.
fn sealed_path(root: &Path, path: &Path) -> bool {
    match path.strip_prefix(root) {
        Ok(rel) => !PLAIN.iter().any(|p| rel.starts_with(p)),
        Err(_) => false,
    }
}

fn seal(key: &[u8; 32], plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    rand::fill(&mut nonce);
    let sealed = XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(&nonce), plain)
        .map_err(|_| io::Error::other("could not encrypt"))?;
    Ok([MAGIC, &nonce[..], &sealed].concat())
}

fn open(key: &[u8; 32], data: &[u8]) -> io::Result<Vec<u8>> {
    let rest = &data[MAGIC.len()..];
    if rest.len() < 24 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an encrypted file is cut short",
        ));
    }
    let (nonce, sealed) = rest.split_at(24);
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), sealed)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "an encrypted file could not be opened (wrong key, or damaged)",
            )
        })
}

/// Reads a file, opening it when it is sealed.
pub fn read<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
    let data = std::fs::read(path)?;
    if !data.starts_with(MAGIC) {
        return Ok(data);
    }
    match STATE.read().unwrap().as_ref() {
        Some((_, key)) => open(key, &data),
        None => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this file is encrypted and the key is not available",
        )),
    }
}

pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// A file's bytes as read from disk, opened when sealed: for code that reads the file itself.
pub fn open_bytes(data: Vec<u8>) -> io::Result<Vec<u8>> {
    if !data.starts_with(MAGIC) {
        return Ok(data);
    }
    match STATE.read().unwrap().as_ref() {
        Some((_, key)) => open(key, &data),
        None => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this file is encrypted and the key is not available",
        )),
    }
}

/// What to put on disk for `contents` at `path`: sealed when [`write`] would seal it. For code
/// that writes the file itself.
pub fn seal_for(path: &Path, contents: &[u8]) -> io::Result<Vec<u8>> {
    if LOCKED
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|root| path.starts_with(root))
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Cortex's data is encrypted and locked (the keychain did not give the key); nothing is saved until it is unlocked"));
    }
    match STATE.read().unwrap().as_ref() {
        Some((root, key)) if sealed_path(root, path) => seal(key, contents),
        _ => Ok(contents.to_vec()),
    }
}

/// Writes a file, sealing it when encryption is on and it is in the settings folder.
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    let path = path.as_ref();
    if LOCKED
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|root| path.starts_with(root))
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Cortex's data is encrypted and locked (the keychain did not give the key); nothing is saved until it is unlocked"));
    }
    let sealed = match STATE.read().unwrap().as_ref() {
        Some((root, key)) if sealed_path(root, path) => Some(seal(key, contents.as_ref())?),
        _ => None,
    };
    match sealed {
        Some(bytes) => std::fs::write(path, bytes),
        None => std::fs::write(path, contents),
    }
}

/// The files under the settings folder that encryption covers.
fn covered(config_dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(config_dir)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || sealed_path(config_dir, e.path()))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        // Only the chat data; the app's own settings and accounts stay where they are.
        .filter(|e| {
            e.path()
                .strip_prefix(config_dir)
                .is_ok_and(|r| r.starts_with("chat"))
        })
        .map(|e| e.into_path())
        .collect()
}

fn is_sealed(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 10];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == MAGIC
}

pub fn status(config_dir: &Path) -> Status {
    let enabled = marker(config_dir).enabled;
    let locked = enabled && (STATE.read().unwrap().is_none() || LOCKED.read().unwrap().is_some());
    let files = covered(config_dir);
    let sealed = files.iter().filter(|p| is_sealed(p)).count();
    Status {
        enabled,
        locked,
        sealed,
        plain: files.len() - sealed,
    }
}

/// Rewrites every covered file through [`write`] (sealing when on, opening when off).
fn rewrite_all(config_dir: &Path) -> Result<usize, String> {
    let mut n = 0;
    for path in covered(config_dir) {
        let data = read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let tmp = path.with_extension("rest-tmp");
        write(&tmp, &data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

fn save_marker(config_dir: &Path, enabled: bool) -> Result<(), String> {
    let p = marker_path(config_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        p,
        serde_json::to_string(&Marker { enabled }).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Turns encryption on with `key` (a new one from [`enable`]); seals what is there.
pub fn enable_with(config_dir: &Path, key: [u8; 32]) -> Result<usize, String> {
    *STATE.write().unwrap() = Some((config_dir.to_path_buf(), key));
    save_marker(config_dir, true)?;
    rewrite_all(config_dir)
}

/// Turns encryption off: every file is written back in the clear.
pub fn disable_files(config_dir: &Path) -> Result<usize, String> {
    // Open with the key, then write plain.
    let key = STATE.read().unwrap().as_ref().map(|(_, k)| *k);
    let mut n = 0;
    for path in covered(config_dir) {
        let data = std::fs::read(&path).map_err(|e| e.to_string())?;
        if !data.starts_with(MAGIC) {
            continue;
        }
        let plain = open(
            &key.ok_or("The key is not available, so the files cannot be decrypted.")?,
            &data,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        let tmp = path.with_extension("rest-tmp");
        std::fs::write(&tmp, plain).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        n += 1;
    }
    *STATE.write().unwrap() = None;
    save_marker(config_dir, false)?;
    Ok(n)
}

fn secrets() -> Result<crate::tools::auth::Secrets, String> {
    Ok(crate::tools::auth::keychain_secrets(
        crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT)?,
    ))
}

/// A new key, kept in the keychain (read back to be sure it is there), then sealing.
pub async fn enable(config_dir: &Path) -> Result<usize, String> {
    if STATE.read().unwrap().is_some() {
        return Ok(0);
    }
    let mut key = [0u8; 32];
    rand::fill(&mut key);
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let store = secrets()?;
    store.store(KEY_NAME, &hex).await?;
    if store.load(KEY_NAME).await?.as_deref() != Some(hex.as_str()) {
        return Err("The key could not be kept in the keychain, so nothing was encrypted.".into());
    }
    let dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || enable_with(&dir, key))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn disable(config_dir: &Path) -> Result<usize, String> {
    let dir = config_dir.to_path_buf();
    let n = tokio::task::spawn_blocking(move || disable_files(&dir))
        .await
        .map_err(|e| e.to_string())??;
    secrets()?.delete(KEY_NAME).await?;
    Ok(n)
}

/// At start, before anything reads the folder: unlock with the keychain's key when on.
pub fn unlock_at_start() {
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        return;
    };
    if !marker(&config_dir).enabled {
        return;
    }
    // The keychain records withdrawn credentials in this folder; it is set once, to the same
    // folder the app sets it to later.
    crate::tools::auth::set_pending_deletes_dir(config_dir.clone());
    let key = tauri::async_runtime::block_on(async { secrets()?.load(KEY_NAME).await });
    if let Err(e) = &key {
        eprintln!("cortex: the encryption key could not be read from the keychain: {e}");
    }
    let parsed = key
        .ok()
        .flatten()
        .and_then(|hex| {
            (0..32)
                .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()
        })
        .and_then(|v| <[u8; 32]>::try_from(v).ok());
    match parsed {
        Some(key) => {
            *STATE.write().unwrap() = Some((config_dir.clone(), key));
            // Anything left in the clear (written before encryption finished) is sealed now.
            if status(&config_dir).plain > 0 {
                if let Err(e) = rewrite_all(&config_dir) {
                    eprintln!("cortex: could not finish encrypting: {e}");
                }
            }
        }
        None => {
            eprintln!("cortex: data on this computer is encrypted, but its key could not be read from the keychain; nothing will be saved until it can be");
            *LOCKED.write().unwrap() = Some(config_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_sealed_and_opened_and_the_folder_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let conv = root.join("chat/conversations/c1.json");
        std::fs::create_dir_all(conv.parent().unwrap()).unwrap();
        std::fs::write(&conv, r#"{"title":"My secret plan"}"#).unwrap();
        let sandbox = root.join("chat/sandbox/c1/data.csv");
        std::fs::create_dir_all(sandbox.parent().unwrap()).unwrap();
        std::fs::write(&sandbox, "a,b").unwrap();
        let outside = tempfile::tempdir().unwrap();

        assert_eq!(
            enable_with(&root, [7u8; 32]).unwrap(),
            1,
            "only the conversation is covered"
        );
        let raw = std::fs::read(&conv).unwrap();
        assert!(raw.starts_with(MAGIC) && !String::from_utf8_lossy(&raw).contains("secret"));
        assert_eq!(
            read_to_string(&conv).unwrap(),
            r#"{"title":"My secret plan"}"#
        );
        assert_eq!(
            std::fs::read_to_string(&sandbox).unwrap(),
            "a,b",
            "sandboxes stay plain"
        );

        // New writes in the folder are sealed; outside it, never.
        let page = root.join("chat/knowledge.json");
        write(&page, "notes").unwrap();
        assert!(std::fs::read(&page).unwrap().starts_with(MAGIC));
        let export = outside.path().join("export.md");
        write(&export, "plain").unwrap();
        assert_eq!(std::fs::read_to_string(&export).unwrap(), "plain");
        let s = status(&root);
        assert!(
            s.enabled && !s.locked && s.sealed == 2 && s.plain == 0,
            "{s:?}"
        );

        assert_eq!(disable_files(&root).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(&conv).unwrap(),
            r#"{"title":"My secret plan"}"#
        );
        assert!(!marker(&root).enabled);
        // Without the key, a sealed file is refused rather than read as garbage.
        std::fs::write(&page, [MAGIC, &[0u8; 40][..]].concat()).unwrap();
        assert_eq!(
            read(&page).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        // Locked: writes into the folder are refused, so nothing replaces sealed data.
        *LOCKED.write().unwrap() = Some(root.clone());
        assert_eq!(
            write(&page, "{}").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        write(&export, "still fine").unwrap();
        *LOCKED.write().unwrap() = None;
    }
}
