//! Download identifiers restrict file-reveal requests to paths recorded by the native webview.
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Mutex,
};
use tauri::{webview::DownloadEvent, Manager, State, Webview};

#[derive(Default)]
pub struct Downloads(Mutex<Tracked>);

#[derive(Default)]
struct Tracked {
    pending: HashMap<String, VecDeque<PathBuf>>,
    completed: VecDeque<(String, PathBuf)>,
}

impl Tracked {
    fn finish(
        &mut self,
        url: &str,
        path: Option<PathBuf>,
        success: bool,
    ) -> Option<(String, PathBuf)> {
        let remembered = self.pending.get_mut(url).and_then(VecDeque::pop_front);
        if self.pending.get(url).is_some_and(VecDeque::is_empty) {
            self.pending.remove(url);
        }
        if !success {
            return None;
        }
        // Tauri omits the saved path from completion events on macOS.
        let path = path.or(remembered)?;
        Some((self.complete(path.clone()), path))
    }

    fn complete(&mut self, path: PathBuf) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.completed.push_back((id.clone(), path));
        while self.completed.len() > 100 {
            self.completed.pop_front();
        }
        id
    }
}

impl Downloads {
    /// Records a file the app saved itself, outside the webview's download handling, so
    /// `reveal_download` accepts it.
    // Only the hosted web app's PDF export calls it, and only on macOS.
    #[cfg_attr(not(all(target_os = "macos", feature = "hosted")), allow(dead_code))]
    pub fn record(&self, path: PathBuf) -> Option<String> {
        Some(self.0.lock().ok()?.complete(path))
    }
}

/// Shows the injected download toast (`download-feedback.js`) for a finished save.
pub fn show_feedback(webview: &Webview, saved: Option<(&str, &std::path::Path)>, success: bool) {
    let payload = serde_json::json!({
        "id": saved.map(|(id, _)| id),
        "name": saved.and_then(|(_, path)| path.file_name()).map(|s| s.to_string_lossy()),
        "success": success,
        "mac": cfg!(target_os = "macos"),
    });
    if webview
        .eval(format!("window.__lumoDownloadFeedback?.({payload});"))
        .is_err()
    {
        log::warn!("Could not display download feedback");
    }
}

pub fn on_download(webview: Webview, event: DownloadEvent<'_>) -> bool {
    let state = webview.state::<Downloads>();
    let Ok(mut tracked) = state.0.lock() else {
        return true;
    };
    match event {
        DownloadEvent::Requested { url, destination } => {
            tracked
                .pending
                .entry(url.to_string())
                .or_default()
                .push_back(destination.clone());
        }
        DownloadEvent::Finished { url, path, success } => {
            let saved = tracked.finish(url.as_str(), path, success);
            drop(tracked);
            show_feedback(
                &webview,
                saved
                    .as_ref()
                    .map(|(id, path)| (id.as_str(), path.as_path())),
                success,
            );
        }
        _ => {}
    }
    true
}

#[tauri::command]
pub async fn reveal_download(
    webview: Webview,
    downloads: State<'_, Downloads>,
    id: String,
) -> Result<(), String> {
    crate::main_window::require(webview.label(), "Downloads require the main webview")?;
    let path = downloads
        .0
        .lock()
        .map_err(|_| "Download state unavailable")?
        .completed
        .iter()
        .find(|(key, _)| key == &id)
        .map(|(_, path)| path.clone())
        .ok_or("Download is no longer available")?;
    tauri::async_runtime::spawn_blocking(move || reveal(path))
        .await
        .map_err(|_| "Could not show downloaded file".to_string())?
}

fn reveal(path: PathBuf) -> Result<(), String> {
    if !path.is_file() {
        return Err("The downloaded file has moved or was removed".into());
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .status()
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(
                    "Finder could not reveal the download",
                ))
            }
        });
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;

        // Explorer requires the path to be quoted separately from the /select, switch.
        let mut argument = std::ffi::OsString::from("/select,\"");
        argument.push(path.as_os_str());
        argument.push("\"");
        std::process::Command::new("explorer.exe")
            .raw_arg(argument)
            .spawn()
            .map(|_| ())
    };
    #[cfg(target_os = "linux")]
    let result = open::that(path.parent().ok_or("Download folder unavailable")?);
    result.map_err(|_| "Could not open the download folder".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_uses_reported_path_or_remembers_macos_destination() {
        let mut state = Tracked::default();
        state.pending.insert(
            "blob:one".into(),
            VecDeque::from([PathBuf::from("/Downloads/a (1).md")]),
        );
        let (_, path) = state.finish("blob:one", None, true).unwrap();
        assert_eq!(path, PathBuf::from("/Downloads/a (1).md"));
        assert!(state.pending.is_empty());
        let (_, path) = state
            .finish("two", Some(PathBuf::from("/actual/b.md")), true)
            .unwrap();
        assert_eq!(path, PathBuf::from("/actual/b.md"));
    }
    #[test]
    fn failure_clears_pending_and_cannot_be_revealed() {
        let mut state = Tracked::default();
        state.pending.insert(
            "one".into(),
            VecDeque::from([PathBuf::from("/Downloads/a.md")]),
        );
        assert!(state.finish("one", None, false).is_none());
        assert!(state.pending.is_empty());
        assert!(state.completed.is_empty());
    }
}
