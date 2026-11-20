//! Errors of the storage layer, typed so that callers can tell what went wrong.
//!
//! Higher layers still often work in `String` errors; `From<StoreError> for String` keeps `?`
//! working there, with the same wording as before. New code should match on the variants where
//! the difference matters: a locked store must not be overwritten with defaults, a missing file
//! is not a damaged one.

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// An id that cannot name a file (ids become file names, so only a safe alphabet is taken).
    #[error("invalid {kind} id `{id}`")]
    InvalidId { kind: &'static str, id: String },
    /// Nothing is saved under this id.
    #[error("no {kind} `{id}`")]
    NotFound { kind: &'static str, id: String },
    /// Encryption at rest is on and its key is not available: nothing may be read or written.
    #[error("{path}: Cortex's data is encrypted and locked (the keychain did not give the key)")]
    Locked { path: PathBuf },
    #[error("could not read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("could not encode {path}: {source}")]
    Encode {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// Something is already saved under this id.
    #[error("a {kind} `{id}` already exists")]
    Exists { kind: &'static str, id: String },
    /// A request the stored data cannot answer, said for the user.
    #[error("{0}")]
    Invalid(String),
}

impl StoreError {
    /// A failed read of `path`: locked when encryption at rest refused it.
    pub fn read(path: &Path, source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::PermissionDenied && is_locked(&source) {
            return Self::Locked {
                path: path.to_path_buf(),
            };
        }
        Self::Read {
            path: path.to_path_buf(),
            source,
        }
    }

    /// A failed write of `path`: locked when encryption at rest refused it.
    pub fn write(path: &Path, source: io::Error) -> Self {
        if source.kind() == io::ErrorKind::PermissionDenied && is_locked(&source) {
            return Self::Locked {
                path: path.to_path_buf(),
            };
        }
        Self::Write {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Locked { .. })
    }
}

/// Encryption at rest refuses with these words (`chat::at_rest`).
fn is_locked(error: &io::Error) -> bool {
    let text = error.to_string();
    text.contains("encrypted and locked") || text.contains("the key is not available")
}

impl From<StoreError> for String {
    fn from(error: StoreError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_store_is_told_apart_from_a_failed_read() {
        let locked = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this file is encrypted and the key is not available",
        );
        assert!(StoreError::read(Path::new("a.json"), locked).is_locked());
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        assert!(!StoreError::read(Path::new("a.json"), denied).is_locked());
        let text: String = StoreError::read(Path::new("a.json"), io::Error::other("disk")).into();
        assert_eq!(text, "could not read a.json: disk");
    }
}
