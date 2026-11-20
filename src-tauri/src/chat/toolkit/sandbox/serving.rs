//! Serving a workspace's files to the window: their type, byte ranges, and safe paths.

use super::*;

pub(super) fn content_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("csv") => "text/csv; charset=utf-8",
        Some("json") => "application/json",
        Some("txt" | "md" | "log") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serves `cortexapp://localhost/sandbox/<conversation>/<file>` so the chat can show a run's
/// charts. Only files inside that conversation's workspace are served.
/// The bytes a `Range: bytes=a-b` header asks for, within a file of `len` bytes.
pub(in crate::chat::toolkit) fn byte_range(header: &str, len: usize) -> Option<(usize, usize)> {
    let spec = header
        .trim()
        .strip_prefix("bytes=")?
        .split(',')
        .next()?
        .trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => (len.saturating_sub(n.parse().ok()?), len.checked_sub(1)?),
        (s, "") => (s.parse().ok()?, len.checked_sub(1)?),
        (s, e) => (
            s.parse().ok()?,
            e.parse::<usize>().ok()?.min(len.checked_sub(1)?),
        ),
    };
    (start <= end && end < len).then_some((start, end))
}

/// Like `serve`, answering a byte range when one is asked for: the media player reads videos
/// in pieces and will not play one served only whole.
pub(crate) fn serve_range(
    config_dir: &Path,
    rest: &str,
    range: Option<&str>,
) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(b"Not found".to_vec())
            .unwrap_or_default()
    };
    let rest = rest.split('?').next().unwrap_or_default();
    let Some((conversation, file)) = rest.split_once('/') else {
        return not_found();
    };
    let file = percent_decode(file);
    if file.is_empty()
        || file
            .split('/')
            .any(|seg| seg.is_empty() || seg == ".." || seg.starts_with('.'))
        || file.contains('\\')
    {
        return not_found();
    }
    let Ok(dir) = workspace(config_dir, conversation) else {
        return not_found();
    };
    let path = dir.join(&file);
    let (Ok(root), Ok(real)) = (dir.canonicalize(), path.canonicalize()) else {
        return not_found();
    };
    if !real.starts_with(&root) {
        return not_found();
    }
    match crate::chat::at_rest::read(&real) {
        Ok(bytes) if range.is_some() => match byte_range(range.unwrap_or_default(), bytes.len()) {
            Some((start, end)) => tauri::http::Response::builder()
                .status(206)
                .header("Content-Type", content_type(&file))
                .header(
                    "Content-Range",
                    format!("bytes {start}-{end}/{}", bytes.len()),
                )
                .header("Accept-Ranges", "bytes")
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .body(bytes[start..=end].to_vec())
                .unwrap_or_default(),
            None => tauri::http::Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{}", bytes.len()))
                .body(Vec::new())
                .unwrap_or_default(),
        },
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Accept-Ranges", "bytes")
            .header("Content-Type", content_type(&file))
            // An SVG could carry script; it is shown as an image, never as a page.
            .header(
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'",
            )
            .header("X-Content-Type-Options", "nosniff")
            .header("Cache-Control", "no-store")
            .body(bytes)
            .unwrap_or_default(),
        Err(_) => not_found(),
    }
}

pub(super) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
