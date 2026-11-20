//! Microsoft's Fluent emoji (<https://github.com/microsoft/fluentui-emoji>, MIT), the set Teams
//! draws, used for every emoji in the interface instead of the system font's.
//!
//! The artwork comes from LobeHub's npm packages of it, which name each image by its code points
//! (`1f44d-1f3fd.webp`), so an emoji finds its picture without a lookup table. A pack is
//! downloaded once from the npm registry, checked against its published SHA-512, and kept in the
//! app's local data folder (not the config folder, so it is not synced or backed up). The page
//! loads the images through the `cortexapp` protocol, at `/emoji/<style>/<name>.<ext>`.

use crate::chat::error::ChatError;
use crate::support::error::StoreError;
use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use serde::Serialize;
use sha2::Digest as _;

pub struct Pack {
    pub style: &'static str,
    pub url: &'static str,
    /// npm's `dist.integrity` for the tarball.
    pub integrity: &'static str,
    pub ext: &'static str,
    pub mime: &'static str,
}

pub const PACKS: &[Pack] = &[
    Pack {
        style: "3d",
        url: "https://registry.npmjs.org/@lobehub/fluent-emoji-3d/-/fluent-emoji-3d-1.1.0.tgz",
        integrity: "sha512-Wlq2lVStTnr8BDpIJ0pEnTKO4LuDxxqTEhTvRtD1DLRF0me7ogeRQjKoehDHXuVJ2Qaecgl5wRBv60lUAG3h7g==",
        ext: "webp",
        mime: "image/webp",
    },
    Pack {
        style: "flat",
        url: "https://registry.npmjs.org/@lobehub/fluent-emoji-flat/-/fluent-emoji-flat-1.1.0.tgz",
        integrity: "sha512-lPYCg8+N56MIj+iOgeBeH6AF0tneXxJCvgi41rOTjAWpA5rU2WUcmU1fiIXouEe8GrSTarWCYLY2ML+rdLRHsg==",
        ext: "svg",
        mime: "image/svg+xml",
    },
];

const MAX_DOWNLOAD: usize = 80 * 1024 * 1024;
const MAX_IMAGE: u64 = 2 * 1024 * 1024;

pub(crate) fn pack(style: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.style == style)
}

pub(in crate::chat::ui) fn dir(root: &Path, style: &str) -> PathBuf {
    root.join("emoji").join(format!("fluent-{style}-1.1.0"))
}

/// An image's name: code points in lowercase hex joined by hyphens.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase() || c == '-')
}

pub(in crate::chat::ui) fn installed(root: &Path, style: &str) -> bool {
    dir(root, style).join(".complete").is_file()
}

/// The image names in an installed pack, without their extension.
pub(crate) fn index(root: &Path, style: &str) -> Vec<String> {
    let Some(pack) = pack(style) else {
        return Vec::new();
    };
    if !installed(root, style) {
        return Vec::new();
    }
    let mut out: Vec<String> = std::fs::read_dir(dir(root, style))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(&format!(".{}", pack.ext))
                .map(str::to_string)
        })
        .filter(|n| valid_name(n))
        .collect();
    out.sort();
    out
}

/// Whether `bytes` match npm's `sha512-<base64>` integrity string.
pub(in crate::chat::ui) fn verify(bytes: &[u8], integrity: &str) -> bool {
    let Some(expected) = integrity.strip_prefix("sha512-") else {
        return false;
    };
    base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(bytes)) == expected
}

/// Writes the pack's images (`package/assets/<name>.<ext>`) from the tarball into `target`;
/// anything else in it is ignored.
pub(in crate::chat::ui) fn extract(
    tgz: &[u8],
    ext: &str,
    target: &Path,
) -> Result<usize, ChatError> {
    let damaged = |e: std::io::Error| ChatError::Invalid(format!("The emoji pack is damaged: {e}"));
    std::fs::create_dir_all(target).map_err(|e| StoreError::write(target, e))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    let mut count = 0;
    for entry in archive.entries().map_err(damaged)? {
        let mut entry = entry.map_err(damaged)?;
        if !entry.header().entry_type().is_file() || entry.size() > MAX_IMAGE {
            continue;
        }
        let path = entry
            .path()
            .map_err(damaged)?
            .to_string_lossy()
            .into_owned();
        let Some(file) = path.strip_prefix("package/assets/") else {
            continue;
        };
        let Some(name) = file.strip_suffix(&format!(".{ext}")) else {
            continue;
        };
        if !valid_name(name) {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(damaged)?;
        let out = target.join(file);
        std::fs::write(&out, bytes).map_err(|e| StoreError::write(&out, e))?;
        count += 1;
    }
    Ok(count)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `ready`, `downloading`, `missing` or `failed`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static RUNNING: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn set_running(style: &str, value: Option<Option<String>>) {
    let mut guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    match value {
        Some(v) => {
            map.insert(style.to_string(), v);
        }
        None => {
            map.remove(style);
        }
    }
}

pub(crate) fn status(root: &Path, style: &str) -> Status {
    if installed(root, style) {
        return Status {
            state: "ready".into(),
            error: None,
        };
    }
    let guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref().and_then(|m| m.get(style)) {
        Some(None) => Status {
            state: "downloading".into(),
            error: None,
        },
        Some(Some(e)) => Status {
            state: "failed".into(),
            error: Some(e.clone()),
        },
        None => Status {
            state: "missing".into(),
            error: None,
        },
    }
}

/// Downloads, checks and unpacks a pack. Marks it ready only once every image is in place.
pub(crate) async fn install(root: &Path, style: &str) -> Result<usize, ChatError> {
    let pack = pack(style).ok_or_else(|| ChatError::Invalid("Unknown emoji style.".into()))?;
    if installed(root, style) {
        return Ok(index(root, style).len());
    }
    set_running(style, Some(None));
    let result = download_and_unpack(root, pack).await;
    match &result {
        Ok(_) => set_running(style, None),
        Err(e) => set_running(style, Some(Some(e.to_string()))),
    }
    result
}

async fn download_and_unpack(root: &Path, pack: &Pack) -> Result<usize, ChatError> {
    const WHAT: &str = "Could not download the emoji";
    let client = crate::support::http::client(std::time::Duration::from_secs(600))
        .map_err(|e| ChatError::network(WHAT, e))?;
    let response = client
        .get(pack.url)
        .send()
        .await
        .map_err(|e| ChatError::network(WHAT, e))?;
    if !response.status().is_success() {
        return Err(ChatError::network(WHAT, response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ChatError::network("The emoji download stopped", e))?;
    if bytes.len() > MAX_DOWNLOAD {
        return Err(ChatError::Invalid(
            "The emoji download is larger than expected.".into(),
        ));
    }
    if !verify(&bytes, pack.integrity) {
        return Err(ChatError::Invalid(
            "The emoji download did not match its published checksum, so it was not used.".into(),
        ));
    }
    let target = dir(root, pack.style);
    let partial = target.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    let ext = pack.ext;
    let partial2 = partial.clone();
    let count = tokio::task::spawn_blocking(move || extract(&bytes, ext, &partial2))
        .await
        .map_err(|e| ChatError::Invalid(e.to_string()))??;
    if count < 1000 {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(ChatError::Invalid(format!(
            "The emoji pack held only {count} images."
        )));
    }
    let complete = partial.join(".complete");
    std::fs::write(&complete, count.to_string()).map_err(|e| StoreError::write(&complete, e))?;
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&partial, &target).map_err(|e| StoreError::write(&target, e))?;
    log::info!("installed the Fluent {} emoji ({count} images)", pack.style);
    Ok(count)
}

/// Serves `<style>/<name>.<ext>` from an installed pack.
pub(crate) fn serve(root: &Path, rest: &str) -> tauri::http::Response<Vec<u8>> {
    let builder = tauri::http::Response::builder();
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap_or_default()
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let Some((style, file)) = rest.split_once('/') else {
        return not_found();
    };
    let Some(pack) = pack(style) else {
        return not_found();
    };
    let Some(name) = file.strip_suffix(&format!(".{}", pack.ext)) else {
        return not_found();
    };
    if !valid_name(name) || !installed(root, style) {
        return not_found();
    }
    match std::fs::read(dir(root, style).join(file)) {
        Ok(bytes) => builder
            .status(200)
            .header("Content-Type", pack.mime)
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes)
            .unwrap_or_default(),
        Err(_) => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (path, bytes) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                // Written straight into the header so that unsafe paths reach the reader.
                let name = header.as_old_mut();
                name.name[..path.len()].copy_from_slice(path.as_bytes());
                header.set_cksum();
                builder.append(&header, *bytes).unwrap();
            }
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn packs_unpack_only_their_images_and_serve_them() {
        let tgz = tarball(&[
            ("package/package.json", b"{}"),
            ("package/assets/1f600.webp", b"RIFFgrin"),
            ("package/assets/1f44d-1f3fd.webp", b"RIFFthumb"),
            ("package/assets/../../evil.webp", b"x"),
            ("package/assets/run.sh", b"x"),
            ("package/assets/1F600.webp", b"x"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out");
        assert_eq!(extract(&tgz, "webp", &target).unwrap(), 2);
        assert!(!dir.path().join("evil.webp").exists());
        std::fs::write(target.join(".complete"), "2").unwrap();
        std::fs::create_dir_all(dir.path().join("emoji")).unwrap();
        std::fs::rename(&target, super::dir(dir.path(), "3d")).unwrap();
        assert_eq!(index(dir.path(), "3d"), ["1f44d-1f3fd", "1f600"]);
        assert_eq!(status(dir.path(), "3d").state, "ready");
        assert_eq!(status(dir.path(), "flat").state, "missing");
        let ok = serve(dir.path(), "3d/1f600.webp?x=1");
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.body(), b"RIFFgrin");
        assert_eq!(ok.headers()["Content-Type"], "image/webp");
        for bad in [
            "3d/../../secret.webp",
            "3d/1f600.png",
            "4d/1f600.webp",
            "3d/1F600.webp",
            "flat/1f600.svg",
        ] {
            assert_eq!(serve(dir.path(), bad).status(), 404, "{bad}");
        }
    }

    #[test]
    fn integrity_is_checked() {
        let bytes = b"emoji";
        let good = format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(bytes))
        );
        assert!(verify(bytes, &good));
        assert!(!verify(b"emojj", &good));
        assert!(!verify(bytes, "sha1-abc"));
    }
}
