//! What an answer had streamed so far, kept beside its conversation while the turn runs.
//!
//! A turn saves its text when it ends; one cut short by the app closing (a crash, a quit, a
//! restart for an update) would lose everything it streamed. The stream writes what it has here
//! every few seconds, the next turn takes it, and until then the window shows it as an answer that
//! was cut off, with a way to continue it.

use std::path::{Path, PathBuf};

use crate::chat::store::{chat_dir, valid_id};
use crate::support::error::StoreError;

fn path(config_dir: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| {
        chat_dir(config_dir)
            .join("partials")
            .join(format!("{id}.txt"))
    })
}

/// Keeps `text` as what the conversation's running answer has streamed so far.
pub fn save(config_dir: &Path, id: &str, text: &str) -> Result<(), StoreError> {
    let Some(path) = path(config_dir, id) else {
        return Err(StoreError::InvalidId {
            kind: "conversation",
            id: id.to_string(),
        });
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| StoreError::write(dir, e))?;
    }
    let tmp = path.with_extension("tmp");
    crate::chat::at_rest::write(&tmp, text).map_err(|e| StoreError::write(&tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| StoreError::write(&path, e))
}

/// What an answer that never finished had streamed, if one did.
pub fn get(config_dir: &Path, id: &str) -> Option<String> {
    crate::chat::at_rest::read_to_string(path(config_dir, id)?)
        .ok()
        .filter(|t| !t.trim().is_empty())
}

/// Takes it: returns it and forgets it, as a new turn does.
pub fn take(config_dir: &Path, id: &str) -> Option<String> {
    let text = get(config_dir, id);
    clear(config_dir, id);
    text
}

/// Forgets it, as a turn that ends does.
pub fn clear(config_dir: &Path, id: &str) {
    if let Some(path) = path(config_dir, id) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_answer_is_kept_until_taken() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(get(dir.path(), "c1"), None);
        save(dir.path(), "c1", "Once upon a").unwrap();
        save(dir.path(), "c1", "Once upon a time").unwrap();
        assert_eq!(get(dir.path(), "c1").as_deref(), Some("Once upon a time"));
        assert_eq!(take(dir.path(), "c1").as_deref(), Some("Once upon a time"));
        assert_eq!(get(dir.path(), "c1"), None);
        assert!(save(dir.path(), "../x", "no").is_err());
    }
}
