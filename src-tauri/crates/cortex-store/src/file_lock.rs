//! One writer at a time for a file that is read, changed and written back whole (an encrypted
//! log, a list on disk), so two changes made at once do not lose one of them.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

/// The files someone is changing, and the signal that one was let go.
struct Held {
    /// A few at most at once, so a list.
    files: Mutex<Vec<PathBuf>>,
    freed: Condvar,
}

// Process-wide: a file has one writer at a time however many parts of the app (the turn, the
// keychain, a command) change it, and they share no other state.
static HELD: Held = Held {
    files: Mutex::new(Vec::new()),
    freed: Condvar::new(),
};

/// The file is the caller's to change until this drops.
#[must_use = "the file is let go as soon as this drops"]
pub struct FileLock(PathBuf);

/// Waits until no one else is changing `path`, then holds it. Blocks the thread: for the short
/// read, change and write of one file, not across an `await`.
pub fn hold(path: &Path) -> FileLock {
    let mut files = HELD.files.lock().unwrap_or_else(|p| p.into_inner());
    while files.iter().any(|held| held == path) {
        files = HELD.freed.wait(files).unwrap_or_else(|p| p.into_inner());
    }
    files.push(path.to_path_buf());
    FileLock(path.to_path_buf())
}

impl Drop for FileLock {
    fn drop(&mut self) {
        HELD.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|held| *held != self.0);
        HELD.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn one_writer_at_a_time_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (path, inside, most) = (path.clone(), inside.clone(), most.clone());
                std::thread::spawn(move || {
                    let _held = hold(&path);
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 1);
        // Another file is not held up by this one.
        let _a = hold(&path);
        let _b = hold(&dir.path().join("other.txt"));
    }
}
