//! Recording a task to teach: the Recording pill, and the review of its steps.

use super::*;

/// Starts recording a task to teach: the main window steps aside for a small Recording pill.
#[tauri::command]
pub fn chat_record_start(
    app: AppHandle,
    recorder: tauri::State<'_, crate::chat::recorder::Recorder>,
) -> Result<(), String> {
    use tauri::Manager;
    recorder.start()?;
    let pill = match app.get_webview_window(crate::chat::recorder::PILL) {
        Some(w) => w,
        None => tauri::WebviewWindowBuilder::new(
            &app,
            crate::chat::recorder::PILL,
            tauri::WebviewUrl::App("record.html".into()),
        )
        .title("Recording")
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .inner_size(380.0, 84.0)
        .build()
        .map_err(|e| e.to_string())?,
    };
    if let Ok(Some(m)) = pill.primary_monitor() {
        let s = m.scale_factor();
        let x = m.position().x + (m.size().width as i32 - (380.0 * s) as i32) / 2;
        let _ = pill.set_position(tauri::PhysicalPosition::new(
            x,
            m.position().y + (24.0 * s) as i32,
        ));
    }
    let _ = pill.show();
    crate::main_window::hide(&app);
    Ok(())
}

#[tauri::command]
pub fn chat_record_status(
    recorder: tauri::State<'_, crate::chat::recorder::Recorder>,
) -> crate::chat::recorder::Status {
    recorder.status()
}

/// Stops the recording (from the pill), brings Cortex back and asks it to review the steps.
#[tauri::command]
pub async fn chat_record_stop(
    app: AppHandle,
    keep: bool,
) -> Result<crate::chat::recorder::Status, String> {
    use tauri::{Emitter, Manager};
    // Stopping waits for one last screenshot: off the main thread.
    let handle = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        let recorder = handle.state::<crate::chat::recorder::Recorder>();
        if keep {
            recorder.stop()
        } else {
            recorder.discard();
            recorder.status()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(pill) = app.get_webview_window(crate::chat::recorder::PILL) {
        let _ = pill.hide();
    }
    crate::main_window::show(&app)?;
    if keep {
        let _ = app.emit("chat://recorded", &status);
    }
    Ok(status)
}

#[tauri::command]
pub fn chat_record_discard(recorder: tauri::State<'_, crate::chat::recorder::Recorder>) {
    recorder.discard();
}
