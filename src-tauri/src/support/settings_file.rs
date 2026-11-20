//! JSON settings files: one way to find, read and save them.
//!
//! A module declares its file once,
//!
//! ```ignore
//! const FILE: SettingsFile<Settings> = SettingsFile::sealed("chat/slack.json");
//! ```
//!
//! and calls `FILE.load(config_dir)` and `FILE.save(config_dir, &settings)`. Saving writes a
//! temporary file and renames it over the old one, so a crash never leaves half a file. Loading
//! returns the default when the file is absent, and also when it cannot be read, after logging
//! why: a damaged settings file must not stop the app, but it must not pass silently either.
//!
//! Whether a file is sealed is part of its format. `sealed` files go through encryption at rest
//! (`chat::at_rest`), which seals any file under the config folder when it is turned on;
//! `plain` files are read and written as they are, because code outside the chat engine, or
//! another program, reads them with plain file I/O. Moving a file from one kind to the other
//! is a data migration, not a refactor.

use std::borrow::Borrow;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// How a settings file is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// Sealed by encryption at rest when it is on.
    Sealed,
    /// Always plain JSON.
    Plain,
}

/// A JSON settings file at a fixed place under the config folder.
pub struct SettingsFile<T> {
    relative: &'static str,
    storage: Storage,
    _value: PhantomData<fn() -> T>,
}

impl<T> SettingsFile<T> {
    /// A file under the config folder that encryption at rest covers.
    pub const fn sealed(relative: &'static str) -> Self {
        Self {
            relative,
            storage: Storage::Sealed,
            _value: PhantomData,
        }
    }

    /// A file that stays plain JSON.
    pub const fn plain(relative: &'static str) -> Self {
        Self {
            relative,
            storage: Storage::Plain,
            _value: PhantomData,
        }
    }

    pub fn path(&self, config_dir: &Path) -> PathBuf {
        config_dir.join(self.relative)
    }
}

impl<T: Serialize + DeserializeOwned + Default> SettingsFile<T> {
    /// The saved value; the default when there is none, or (logged) when it cannot be read.
    pub fn load(&self, config_dir: &Path) -> T {
        self.try_load(config_dir).unwrap_or_else(|e| {
            log::warn!("{e}; using the defaults");
            T::default()
        })
    }

    /// The saved value, the default when there is none, or why it could not be read.
    pub fn try_load(&self, config_dir: &Path) -> Result<T, String> {
        Ok(read_json(&self.path(config_dir), self.storage)?.unwrap_or_default())
    }

    /// Saves `value`: the file's type itself, or what it borrows as (a `&[T]` for a `Vec<T>`).
    pub fn save<V>(&self, config_dir: &Path, value: &V) -> Result<(), String>
    where
        T: Borrow<V>,
        V: Serialize + ?Sized,
    {
        write_json(&self.path(config_dir), self.storage, value)
    }
}

/// Reads a JSON file: `None` when it does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path, storage: Storage) -> Result<Option<T>, String> {
    let text = match storage {
        Storage::Sealed => crate::chat::at_rest::read_to_string(path),
        Storage::Plain => std::fs::read_to_string(path),
    };
    let text = match text {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("could not parse {}: {e}", path.display()))
}

/// Writes a JSON file through a temporary file, so a crash never leaves half of it.
pub fn write_json<T: Serialize + ?Sized>(
    path: &Path,
    storage: Storage,
    value: &T,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    match storage {
        Storage::Sealed => crate::chat::at_rest::write(&tmp, text),
        Storage::Plain => std::fs::write(&tmp, text),
    }
    .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Prefs {
        name: String,
        count: u32,
    }

    const SEALED: SettingsFile<Prefs> = SettingsFile::sealed("chat/test-prefs.json");
    const PLAIN: SettingsFile<Prefs> = SettingsFile::plain("accounts/x/test-prefs.json");

    #[test]
    fn absent_is_the_default_and_saved_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        for file in [&SEALED, &PLAIN] {
            assert_eq!(file.load(dir.path()), Prefs::default());
            let value = Prefs {
                name: "a".into(),
                count: 3,
            };
            file.save(dir.path(), &value).unwrap();
            assert_eq!(file.load(dir.path()), value);
            assert!(!file.path(dir.path()).with_extension("json.tmp").exists());
        }
    }

    #[test]
    fn an_unreadable_file_is_reported_and_loads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = PLAIN.path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(PLAIN
            .try_load(dir.path())
            .unwrap_err()
            .contains("could not parse"));
        assert_eq!(PLAIN.load(dir.path()), Prefs::default());
    }

    #[test]
    fn plain_files_are_plain_json() {
        let dir = tempfile::tempdir().unwrap();
        PLAIN
            .save(
                dir.path(),
                &Prefs {
                    name: "b".into(),
                    count: 1,
                },
            )
            .unwrap();
        let text = std::fs::read_to_string(PLAIN.path(dir.path())).unwrap();
        assert!(text.contains("\"name\": \"b\""));
    }
}
