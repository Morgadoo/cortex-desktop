//! The settings page, a child webview stacked over the web app inside the main window.

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, State, WebviewUrl};

use super::load_device_settings;
use crate::main_window::{self, set_page_visible};
use crate::{updater, webview};

pub const LABEL: &str = "settings-overlay";

/// Keep the settings child webview pinned to the full main window. Without
/// `auto_resize`, it keeps the size from first creation and the HTML layout
/// appears frozen while the outer window changes.
pub fn sync_bounds(app_handle: &AppHandle) -> Result<(), String> {
    let Some(overlay) = app_handle.get_webview(LABEL) else {
        return Ok(());
    };

    let window = app_handle
        .get_window(main_window::LABEL)
        .ok_or_else(|| "Main window not found".to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;

    overlay
        .set_position(LogicalPosition::new(0.0, 0.0))
        .map_err(|e| e.to_string())?;
    overlay.set_size(size).map_err(|e| e.to_string())?;
    overlay.set_auto_resize(true).map_err(|e| e.to_string())?;

    Ok(())
}

fn ensure_overlay(app_handle: &AppHandle) -> Result<(), String> {
    if app_handle.get_webview(LABEL).is_none() {
        let window = app_handle
            .get_window(main_window::LABEL)
            .ok_or_else(|| "Main window not found".to_string())?;
        let size = window.inner_size().map_err(|e| e.to_string())?;

        let builder = WebviewBuilder::new(LABEL, WebviewUrl::App("settings.html".into()))
            .auto_resize()
            .on_navigation({
                let app_handle = app_handle.clone();
                move |url| webview::handle_navigation(&app_handle, url)
            })
            .on_new_window({
                let app_handle = app_handle.clone();
                move |url, _features| webview::handle_new_window(&app_handle, &url)
            });

        window
            .add_child(builder, LogicalPosition::new(0.0, 0.0), size)
            .map_err(|e| e.to_string())?;

        if let Some(overlay) = app_handle.get_webview(LABEL) {
            set_page_visible(&overlay, false)?;
        }
    }

    sync_bounds(app_handle)
}

pub fn show(app_handle: &AppHandle) -> Result<(), String> {
    if updater::is_required(app_handle) {
        return Ok(());
    }
    main_window::show(app_handle)?;

    // The native chat page has its own settings dialog; the overlay belongs to the hosted app.
    if !crate::HOSTED_WEB_APP && !crate::e2e_testing_enabled() {
        use tauri::Emitter;
        return app_handle
            .get_webview(main_window::LABEL)
            .ok_or_else(|| "Lumo window is not open".to_string())?
            .emit("chat://open-settings", ())
            .map_err(|e| e.to_string());
    }

    ensure_overlay(app_handle)?;

    if let Some(main_webview) = app_handle.get_webview(main_window::LABEL) {
        set_page_visible(&main_webview, false)?;
    }

    let overlay = app_handle
        .get_webview(LABEL)
        .ok_or_else(|| "Settings view not found".to_string())?;

    set_page_visible(&overlay, true)?;
    overlay.set_focus().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub fn close_settings_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(overlay) = app_handle.get_webview(LABEL) {
        set_page_visible(&overlay, false)?;
    }

    if let Some(main_webview) = app_handle.get_webview(main_window::LABEL) {
        set_page_visible(&main_webview, true)?;
        main_webview.set_focus().map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[derive(serde::Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsOpenSource {
    Hotkey,
    AppUi,
}

#[derive(Default)]
pub struct PendingSettingsTab(std::sync::Mutex<Option<String>>);

#[tauri::command]
pub fn take_pending_settings_tab(state: State<'_, PendingSettingsTab>) -> Option<String> {
    state.0.lock().ok().and_then(|mut slot| slot.take())
}

/// WARNING: must stay async. A synchronous command runs on the main thread as opposed to
/// a worker thread, and `add_child` behind `show` deadlocks on Windows
#[tauri::command]
pub async fn open_settings_window(
    app_handle: AppHandle,
    source: SettingsOpenSource,
    tab: Option<String>,
) -> Result<(), String> {
    if source == SettingsOpenSource::Hotkey
        && !load_device_settings(&app_handle)?.settings_hotkey_enabled
    {
        return Ok(());
    }

    if let Some(tab) = &tab {
        if let Ok(mut slot) = app_handle.state::<PendingSettingsTab>().0.lock() {
            *slot = Some(tab.clone());
        }
    }

    show(&app_handle)?;

    if let Some(tab) = tab {
        let _ = app_handle.emit("settings:show-tab", tab);
    }

    Ok(())
}
