//! The app itself: notifications, windows, app links and files the page saves.

use super::*;

/// A desktop notification, e.g. when a reply finishes while the window is in the background.
#[tauri::command]
pub fn chat_notify(app: AppHandle, title: String, body: String) {
    let body: String = crate::support::text::clip(&body, 180);
    crate::chat::engine::notify(&app, title.trim(), body.trim());
}

/// Brings the main window forward (the wake word heard while Cortex was in the background).
#[tauri::command]
pub fn chat_show_window(app: AppHandle) -> Result<(), String> {
    crate::main_window::show(&app)
}

/// Opens a conversation in a window of its own (LobeHub's "open in new window"), or brings its
/// window forward when it is open already. The window runs the same page, told which chat to show.
#[tauri::command]
pub fn chat_open_chat_window(app: AppHandle, id: String) -> Result<(), String> {
    use tauri::Manager as _;
    if !store::valid_id(&id) {
        return Err("invalid conversation id".into());
    }
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    let label = format!(
        "cortex-chat-{}",
        id.chars()
            .filter(char::is_ascii_alphanumeric)
            .take(32)
            .collect::<String>()
    );
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    let handle = app.clone();
    let handle2 = app.clone();
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App(crate::webview::APP_PAGE.into()),
    )
    .title(if conversation.title.is_empty() {
        "Cortex".to_string()
    } else {
        conversation.title.clone()
    })
    .inner_size(900.0, 780.0)
    .min_inner_size(480.0, 420.0)
    .initialization_script(format!(
        "window.__CORTEX_SOLO_CHAT = {};",
        serde_json::to_string(&id).map_err(|e| e.to_string())?
    ))
    .disable_drag_drop_handler()
    .on_navigation(move |url| crate::webview::handle_navigation(&handle, url))
    .on_new_window(move |url, _features| crate::webview::handle_new_window(&handle2, &url))
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The app link (cortex://…) waiting for the page, if any.
#[tauri::command]
pub fn chat_take_link(
    pending: State<'_, crate::chat::links::PendingLink>,
) -> Option<crate::chat::links::Link> {
    pending.take()
}

/// Saves bytes the page made (an image of a conversation) where the user picks.
#[tauri::command]
pub async fn chat_save_bytes(app: AppHandle, name: String, data: String) -> Result<bool, String> {
    use base64::Engine as _;
    use tauri_plugin_dialog::DialogExt;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    let ext = name.rsplit('.').next().unwrap_or("bin").to_string();
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(&name)
            .add_filter(ext.to_uppercase(), &[ext.as_str()]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(true)
}
