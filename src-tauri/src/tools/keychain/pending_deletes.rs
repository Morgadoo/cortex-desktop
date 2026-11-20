//! Keychain entries whose delete failed, recorded on disk so they can never be loaded again.
//!
//! A credential sometimes has to be withdrawn while the keychain is locked or not yet running,
//! e.g. when an MCP server is pointed at a new URL. Refusing the change would block every edit
//! on a machine without a keyring, and skipping the delete would let the old secret load later
//! and reach the new destination. Instead the key is recorded here, and a load deletes it first,
//! refusing to return anything until that delete succeeds.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILE_NAME: &str = "pending_secret_deletes.json";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Called once at startup with the app config directory. Until then nothing can be recorded or
/// checked, so a failed delete and every load are reported as errors.
pub fn set_root(config_dir: PathBuf) {
    let _ = ROOT.set(config_dir);
}

fn file() -> Option<PathBuf> {
    ROOT.get().map(|dir| dir.join(FILE_NAME))
}

pub(crate) fn store(key: &str, store: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    store_in(file().as_deref(), key, store)
}

pub(crate) fn load(
    key: &str,
    delete: impl FnOnce() -> Result<(), String>,
    load: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    load_in(file().as_deref(), key, delete, load)
}

pub(crate) fn delete(key: &str, delete: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    delete_in(file().as_deref(), key, delete)
}

/// Still pending after a successful store, the new secret would be deleted on load, which costs
/// a re-entry but never leaks; so a failed clear is reported rather than ignored.
fn store_in(
    path: Option<&Path>,
    key: &str,
    store: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    store()?;
    path.map_or(Ok(()), |path| clear_in(path, key))
}

fn load_in(
    path: Option<&Path>,
    key: &str,
    delete: impl FnOnce() -> Result<(), String>,
    load: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    // Without a location, keys recorded by an earlier run can't be checked.
    let path = path.ok_or("could not check for withdrawn credentials")?;
    if !is_pending_in(path, key)? {
        return load();
    }
    delete().map_err(|e| format!("a withdrawn credential could not be cleared yet: {e}"))?;
    let _ = clear_in(path, key);
    Ok(None)
}

fn delete_in(
    path: Option<&Path>,
    key: &str,
    delete: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match (delete(), path) {
        (Ok(()), Some(path)) => {
            let _ = clear_in(path, key);
            Ok(())
        }
        (Ok(()), None) => Ok(()),
        (Err(e), Some(path)) => record_in(path, key)
            .map_err(|record| format!("{e}; and could not defer the delete: {record}")),
        (Err(e), None) => Err(e),
    }
}

fn read(path: &Path) -> Result<BTreeSet<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("could not read pending secret deletes: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(format!("could not read pending secret deletes: {e}")),
    }
}

fn write(path: &Path, keys: &BTreeSet<String>) -> Result<(), String> {
    if keys.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("could not update pending secret deletes: {e}"))
            }
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(keys).map_err(|e| e.to_string())?;
    std::fs::write(path, content)
        .map_err(|e| format!("could not record pending secret delete: {e}"))
}

fn record_in(path: &Path, key: &str) -> Result<(), String> {
    let _guard = cortex_store::file_lock::hold(path);
    let mut keys = read(path)?;
    if keys.insert(key.to_string()) {
        write(path, &keys)?;
    }
    Ok(())
}

/// An unreadable file counts as an error rather than "not pending": the caller must not load.
fn is_pending_in(path: &Path, key: &str) -> Result<bool, String> {
    let _guard = cortex_store::file_lock::hold(path);
    Ok(read(path)?.contains(key))
}

fn clear_in(path: &Path, key: &str) -> Result<(), String> {
    let _guard = cortex_store::file_lock::hold(path);
    let mut keys = read(path)?;
    if keys.remove(key) {
        write(path, &keys)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_key_stays_pending_until_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);

        assert!(!is_pending_in(&path, "connector:u:mcp-a:token").unwrap());
        record_in(&path, "connector:u:mcp-a:token").unwrap();
        record_in(&path, "connector:u:mcp-a:token").unwrap();
        assert!(is_pending_in(&path, "connector:u:mcp-a:token").unwrap());
        assert!(!is_pending_in(&path, "connector:u:mcp-b:token").unwrap());

        clear_in(&path, "connector:u:mcp-a:token").unwrap();
        assert!(!is_pending_in(&path, "connector:u:mcp-a:token").unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn a_secret_whose_delete_failed_is_never_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = Some(dir.path().join(FILE_NAME));
        let path = path.as_deref();
        let key = "connector:u:mcp-a:token";
        let locked = || Err("secure storage is unavailable".to_string());
        let old_secret = || Ok(Some("token-for-a".to_string()));

        delete_in(path, key, locked).unwrap();

        assert!(load_in(path, key, locked, old_secret).is_err());
        assert_eq!(load_in(path, key, || Ok(()), old_secret).unwrap(), None);
        assert_eq!(
            load_in(path, key, locked, old_secret).unwrap().as_deref(),
            Some("token-for-a"),
            "once the delete went through the key is no longer pending"
        );
    }

    #[test]
    fn storing_a_new_secret_lifts_the_pending_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = Some(dir.path().join(FILE_NAME));
        let path = path.as_deref();
        let key = "connector:u:mcp-a:token";

        delete_in(path, key, || Err("locked".into())).unwrap();
        store_in(path, key, || Ok(())).unwrap();

        assert!(!is_pending_in(path.unwrap(), key).unwrap());
    }

    #[test]
    fn a_failed_delete_is_an_error_when_it_cannot_be_recorded() {
        assert!(delete_in(None, "connector:u:mcp-a:token", || Err("locked".into())).is_err());
    }

    #[test]
    fn nothing_loads_when_earlier_deletes_cannot_be_checked() {
        let loaded = load_in(
            None,
            "connector:u:mcp-a:token",
            || Ok(()),
            || Ok(Some("token-for-a".to_string())),
        );
        assert!(loaded.is_err());
    }

    #[test]
    fn an_unreadable_ledger_is_an_error_not_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "not json").unwrap();
        assert!(is_pending_in(&path, "connector:u:mcp-a:token").is_err());
    }
}
