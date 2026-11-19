//! Carries data over from the app's former identity. Cortex Desktop was Lumo Desktop, whose
//! bundle identifier `me.proton.lumo-app` named its folders and its keychain service. On the first
//! launch under the new identifier the old folders are moved into place; keychain secrets move
//! one at a time, the first time each is read (see `tools::keychain::keyring_secrets`).

use std::path::{Path, PathBuf};

/// The bundle identifier and keychain service before the rename.
pub const LEGACY_ID: &str = "me.proton.lumo-app";
/// The current bundle identifier; matches `identifier` in tauri.conf.json.
pub const ID: &str = "io.github.morgadoo.cortex-desktop";

/// Folders under which Tauri, the webview and the keychain fallback keep a folder named after the
/// bundle identifier, per platform.
fn base_dirs() -> Vec<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = env("HOME");
    let mut dirs: Vec<PathBuf> = match std::env::consts::OS {
        "macos" => [
            "Library/Application Support",
            "Library/Caches",
            "Library/Logs",
            "Library/WebKit",
        ]
        .iter()
        .filter_map(|sub| home.as_ref().map(|h| h.join(sub)))
        .collect(),
        "windows" => [env("APPDATA"), env("LOCALAPPDATA")]
            .into_iter()
            .flatten()
            .collect(),
        _ => [
            env("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|h| h.join(".config"))),
            env("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share"))),
            env("XDG_CACHE_HOME").or_else(|| home.as_ref().map(|h| h.join(".cache"))),
        ]
        .into_iter()
        .flatten()
        .collect(),
    };
    dirs.dedup();
    dirs
}

/// Moves each `<base>/me.proton.lumo-app` to `<base>/io.github.morgadoo.cortex-desktop` when only
/// the old one exists. Runs before anything opens the app's folders, and is a no-op afterwards.
pub fn migrate_folders() {
    for base in base_dirs() {
        if let Err(e) = migrate_folder(&base) {
            eprintln!(
                "cortex: could not move {} to {}: {e}",
                base.join(LEGACY_ID).display(),
                base.join(ID).display()
            );
        }
    }
}

fn migrate_folder(base: &Path) -> std::io::Result<()> {
    let old = base.join(LEGACY_ID);
    let new = base.join(ID);
    if !old.is_dir() || new.exists() {
        return Ok(());
    }
    std::fs::rename(&old, &new)?;
    eprintln!("cortex: moved {} to {}", old.display(), new.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_folder_moves_when_the_new_one_is_absent() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join(LEGACY_ID).join("chat")).unwrap();
        std::fs::write(base.path().join(LEGACY_ID).join("chat/a.json"), "{}").unwrap();
        migrate_folder(base.path()).unwrap();
        assert!(!base.path().join(LEGACY_ID).exists());
        assert_eq!(
            std::fs::read_to_string(base.path().join(ID).join("chat/a.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn an_existing_new_folder_is_never_overwritten() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join(LEGACY_ID)).unwrap();
        std::fs::create_dir_all(base.path().join(ID)).unwrap();
        std::fs::write(base.path().join(ID).join("keep"), "new").unwrap();
        migrate_folder(base.path()).unwrap();
        assert!(base.path().join(LEGACY_ID).exists());
        assert_eq!(
            std::fs::read_to_string(base.path().join(ID).join("keep")).unwrap(),
            "new"
        );
    }

    #[test]
    fn the_identifier_matches_the_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], ID);
        assert_eq!(crate::settings::KEYCHAIN_SERVICE, ID);
    }
}
