//! The system's file dialogs, awaited: what the user chose, or `None` when they cancelled.
//!
//! The dialog plugin answers through a callback; each function here turns that into a value, so
//! a command reads `let Some(path) = dialogs::save_file(builder).await? else { … }`.

use tauri_plugin_dialog::{FileDialogBuilder, FilePath};
use tokio::sync::oneshot;

type Builder = FileDialogBuilder<tauri::Wry>;

async fn answer<T>(open: impl FnOnce(oneshot::Sender<T>)) -> Result<T, String> {
    let (tx, rx) = oneshot::channel();
    open(tx);
    rx.await.map_err(|e| e.to_string())
}

/// Where to save a file.
pub async fn save_file(dialog: Builder) -> Result<Option<FilePath>, String> {
    answer(|tx| {
        dialog.save_file(move |path| {
            let _ = tx.send(path);
        })
    })
    .await
}

/// One file to open.
pub async fn pick_file(dialog: Builder) -> Result<Option<FilePath>, String> {
    answer(|tx| {
        dialog.pick_file(move |path| {
            let _ = tx.send(path);
        })
    })
    .await
}

/// Files to open.
pub async fn pick_files(dialog: Builder) -> Result<Option<Vec<FilePath>>, String> {
    answer(|tx| {
        dialog.pick_files(move |paths| {
            let _ = tx.send(paths);
        })
    })
    .await
}

/// A folder.
pub async fn pick_folder(dialog: Builder) -> Result<Option<FilePath>, String> {
    answer(|tx| {
        dialog.pick_folder(move |path| {
            let _ = tx.send(path);
        })
    })
    .await
}
