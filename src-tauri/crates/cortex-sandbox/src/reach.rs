//! What a tool may reach: files beneath its workspace (never through a link), and network addresses that are not private.

use super::*;

/// Adds a chunk of a response as it arrives, and stops the download the moment it passes `cap`:
/// a large response is refused without being held in memory.
pub(super) fn push_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), String> {
    if body.len() + chunk.len() > cap {
        return Err(format!("denied: the response is larger than {cap} bytes"));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// A script's path, if it is plain and relative: no root, no `..`, no `.`.
pub(super) fn plain_relative(path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if path.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, PathPart::Normal(_)))
    {
        return Err(format!("denied: `{path}` is not a plain relative path"));
    }
    Ok(relative.to_path_buf())
}

/// Why a file in the workspace could not be reached.
#[derive(Debug)]
pub(super) enum Reach {
    Link,
    NotFound,
    TooLarge,
    NotAFile,
    Io(std::io::Error),
}

impl Reach {
    pub(super) fn describe(self, path: &str, verb: &str) -> String {
        match self {
            Reach::Link => format!(
                "denied: `{path}` goes through a link, which may lead outside the folders this tool may {verb}, or that leaves the workspace"
            ),
            Reach::NotFound => format!("not found: `{path}`"),
            Reach::TooLarge => format!("denied: `{path}` is larger than {MAX_READ_BYTES} bytes"),
            Reach::NotAFile => format!("denied: `{path}` is a folder, not a file"),
            Reach::Io(e) => format!("could not {verb} `{path}`: {e}"),
        }
    }
}

/// Files under the workspace, reached without following links: each folder is opened from the
/// one above it (`openat` with `O_NOFOLLOW`), so a link anywhere on the way, or one swapped in
/// mid-call, is refused rather than followed.
#[cfg(unix)]
pub(super) mod beneath {
    use super::Reach;
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    fn name(part: &std::ffi::OsStr) -> Result<CString, Reach> {
        CString::new(part.as_bytes()).map_err(|_| Reach::NotFound)
    }

    fn error() -> Reach {
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ELOOP) | Some(libc::ENOTDIR) => Reach::Link,
            Some(libc::ENOENT) => Reach::NotFound,
            Some(libc::EISDIR) => Reach::NotAFile,
            _ => Reach::Io(e),
        }
    }

    fn open_root(root: &Path) -> Result<OwnedFd, Reach> {
        let root = CString::new(root.as_os_str().as_bytes()).map_err(|_| Reach::NotFound)?;
        // SAFETY: a valid C string; the returned descriptor is owned below.
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error());
        }
        // SAFETY: `fd` is a fresh descriptor nobody else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// The folder `parts` names under `dir`, made where missing when `create`.
    fn folder(dir: OwnedFd, parts: &[Component], create: bool) -> Result<OwnedFd, Reach> {
        let mut dir = dir;
        for part in parts {
            let Component::Normal(part) = part else {
                return Err(Reach::NotFound);
            };
            let part = name(part)?;
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: valid descriptor and C string.
            let mut fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
            if fd < 0
                && create
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                // SAFETY: as above; mkdirat makes a folder, never follows a link.
                if unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o755) } != 0 {
                    return Err(error());
                }
                // SAFETY: as above.
                fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
            }
            if fd < 0 {
                return Err(error());
            }
            // SAFETY: `fd` is fresh and owned here.
            dir = unsafe { OwnedFd::from_raw_fd(fd) };
        }
        Ok(dir)
    }

    fn file(dir: &OwnedFd, part: &Component, flags: i32) -> Result<File, Reach> {
        let Component::Normal(part) = part else {
            return Err(Reach::NotFound);
        };
        let part = name(part)?;
        // SAFETY: valid descriptor and C string; O_NOFOLLOW refuses a link in the file's place.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                // O_NONBLOCK: a FIFO in the workspace returns at once instead of blocking a
                // thread forever; anything but a plain file is then refused.
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o644 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(error());
        }
        // SAFETY: `fd` is fresh and owned by the File.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub fn read(root: &Path, relative: &Path, max: u64) -> Result<Vec<u8>, Reach> {
        let parts: Vec<Component> = relative.components().collect();
        let (last, folders) = parts.split_last().ok_or(Reach::NotFound)?;
        let dir = folder(open_root(root)?, folders, false)?;
        let mut f = file(&dir, last, libc::O_RDONLY)?;
        let meta = f.metadata().map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        // A hard link names a file that may live in a folder the tool was not granted.
        if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
            return Err(Reach::Link);
        }
        if meta.len() > max {
            return Err(Reach::TooLarge);
        }
        let mut out = Vec::new();
        Read::take(&mut f, max + 1)
            .read_to_end(&mut out)
            .map_err(Reach::Io)?;
        if out.len() as u64 > max {
            return Err(Reach::TooLarge);
        }
        Ok(out)
    }

    pub fn write(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), Reach> {
        let parts: Vec<Component> = relative.components().collect();
        let (last, folders) = parts.split_last().ok_or(Reach::NotFound)?;
        let dir = folder(open_root(root)?, folders, true)?;
        // Not truncated on open: a hard-linked file is refused before anything in it changes.
        let mut f = file(&dir, last, libc::O_WRONLY | libc::O_CREAT)?;
        let meta = f.metadata().map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
            return Err(Reach::Link);
        }
        f.set_len(0).map_err(Reach::Io)?;
        f.write_all(bytes).map_err(Reach::Io)
    }
}

/// Elsewhere, the same rules by path: each level checked for a link before it is used.
#[cfg(not(unix))]
pub(super) mod beneath {
    use super::Reach;
    use std::path::Path;

    fn walk(root: &Path, relative: &Path, create: bool) -> Result<std::path::PathBuf, Reach> {
        let mut at = root.to_path_buf();
        let parts: Vec<_> = relative.components().collect();
        for (i, part) in parts.iter().enumerate() {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(m) if m.file_type().is_symlink() => return Err(Reach::Link),
                Ok(_) => {}
                Err(_) if create && i + 1 < parts.len() => {
                    std::fs::create_dir(&at).map_err(Reach::Io)?
                }
                Err(_) if create => {}
                Err(_) => return Err(Reach::NotFound),
            }
        }
        Ok(at)
    }

    pub fn read(root: &Path, relative: &Path, max: u64) -> Result<Vec<u8>, Reach> {
        let at = walk(root, relative, false)?;
        let meta = std::fs::metadata(&at).map_err(Reach::Io)?;
        if !meta.is_file() {
            return Err(Reach::NotAFile);
        }
        if meta.len() > max {
            return Err(Reach::TooLarge);
        }
        std::fs::read(at).map_err(Reach::Io)
    }

    pub fn write(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), Reach> {
        let at = walk(root, relative, true)?;
        if at.is_dir() {
            return Err(Reach::NotAFile);
        }
        std::fs::write(at, bytes).map_err(Reach::Io)
    }
}

/// The first public address a name resolved to; none when every one is private.
pub(super) fn checked_address(
    resolved: impl IntoIterator<Item = SocketAddr>,
) -> Option<SocketAddr> {
    resolved.into_iter().find(|a| !is_private(a.ip()))
}

/// A client that reaches `host` only at `address`, the one that was checked: the name is not
/// looked up again, so it cannot be pointed elsewhere between the check and the request.
pub(super) fn pinned_client(host: &str, address: SocketAddr) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        // A proxy would look the name up again, past the check and the pin.
        .no_proxy()
        .resolve(host, address)
        .build()
        .map_err(|e| e.to_string())
}
