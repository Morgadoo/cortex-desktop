//! Downloading the built-in models' weights, with progress, cancelling and removal.

use super::*;
use crate::chat::error::ChatError;

// Downloads

pub(super) fn downloads() -> &'static Mutex<Vec<String>> {
    &super::LOCAL_MODELS.downloads
}

pub(super) fn cancelled() -> &'static Mutex<Vec<String>> {
    &super::LOCAL_MODELS.cancelled
}

pub(crate) fn cancel(id: &str) {
    if let Ok(mut c) = cancelled().lock() {
        c.push(id.to_string());
    }
}

/// Where a file comes from. `CORTEX_MODELS_URL` points at a mirror with the same layout.
pub(super) fn file_url(repo: &str, path: &str) -> String {
    let base =
        std::env::var("CORTEX_MODELS_URL").unwrap_or_else(|_| "https://huggingface.co".into());
    format!("{}/{repo}/resolve/main/{path}", base.trim_end_matches('/'))
}

/// Downloads a model's files, reporting `(bytes done, bytes in all)` as it goes.
pub(crate) async fn download(
    root: &Path,
    id: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), ChatError> {
    use futures::StreamExt as _;
    let e = entry(id)
        .ok_or_else(|| format!("Unknown model `{id}`."))
        .map_err(ChatError::Invalid)?;
    {
        let mut d = downloads().lock().map_err(ChatError::internal)?;
        if d.iter().any(|x| x == id) {
            return Err(ChatError::Invalid("It is already downloading.".into()));
        }
        d.push(id.to_string());
    }
    let result = async {
        let dir = model_dir(root, id)?;
        std::fs::create_dir_all(&dir).map_err(ChatError::internal)?;
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(ChatError::internal)?;
        // Sizes first, for one progress bar over all files.
        let mut sizes = Vec::new();
        for (repo, path, _) in e.files {
            let head = client
                .head(file_url(repo, path))
                .send()
                .await
                .map_err(|err| ChatError::Invalid(format!("Hugging Face did not answer: {err}")))?;
            sizes.push(
                head.headers()
                    .get("x-linked-size")
                    .or_else(|| head.headers().get("content-length"))
                    .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
                    .unwrap_or(0),
            );
        }
        let total: u64 = sizes.iter().sum();
        let mut done = 0u64;
        for (i, (repo, path, name)) in e.files.iter().enumerate() {
            let target = dir.join(name);
            if target.exists()
                && std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0) == sizes[i]
                && sizes[i] > 0
            {
                done += sizes[i];
                progress(done, total);
                continue;
            }
            let r = client
                .get(file_url(repo, path))
                .send()
                .await
                .map_err(|err| ChatError::Invalid(format!("The download failed: {err}")))?;
            if !r.status().is_success() {
                return Err(ChatError::Invalid(format!(
                    "Hugging Face answered {} for {path}.",
                    r.status()
                )));
            }
            let part = dir.join(format!("{name}.part"));
            let mut out = std::fs::File::create(&part).map_err(ChatError::internal)?;
            let mut stream = r.bytes_stream();
            let mut last = std::time::Instant::now();
            while let Some(chunk) = stream.next().await {
                if cancelled().lock().is_ok_and(|c| c.iter().any(|x| x == id)) {
                    drop(out);
                    let _ = std::fs::remove_file(&part);
                    return Err(ChatError::Invalid("Download cancelled.".into()));
                }
                let chunk = chunk
                    .map_err(|err| ChatError::Invalid(format!("The download stopped: {err}")))?;
                out.write_all(&chunk).map_err(ChatError::internal)?;
                done += chunk.len() as u64;
                if last.elapsed().as_millis() > 250 {
                    last = std::time::Instant::now();
                    progress(done, total);
                }
            }
            out.flush().map_err(ChatError::internal)?;
            drop(out);
            std::fs::rename(&part, &target).map_err(ChatError::internal)?;
        }
        std::fs::write(dir.join(".complete"), b"").map_err(ChatError::internal)?;
        progress(total, total);
        Ok(())
    }
    .await;
    if let Ok(mut d) = downloads().lock() {
        d.retain(|x| x != id);
    }
    if let Ok(mut c) = cancelled().lock() {
        c.retain(|x| x != id);
    }
    result
}

pub(crate) fn remove(root: &Path, id: &str) -> Result<(), ChatError> {
    unload(id);
    match std::fs::remove_dir_all(model_dir(root, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ChatError::Invalid(e.to_string())),
    }
}
