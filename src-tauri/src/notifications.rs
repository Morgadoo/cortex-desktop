//! Native desktop notifications exposed to the authenticated main webview.

use tauri::{AppHandle, State, Webview};

use super::account::{AccountId, ActiveAccount};

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, test))]
mod windows;

#[cfg(target_os = "linux")]
use self::linux as platform;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(windows)]
use self::windows as platform;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPermission {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotDetermined,
    Denied,
    Granted,
}

fn require_main(webview: &Webview) -> Result<(), String> {
    crate::main_window::require(webview.label(), "notifications require the main webview")
}

async fn permission(app: &AppHandle, request: bool) -> Result<bool, String> {
    match platform::permission_state(app).await? {
        NotificationPermission::Granted => Ok(true),
        NotificationPermission::NotDetermined if request => platform::request_permission(app).await,
        _ => Ok(false),
    }
}

async fn show(app: &AppHandle, title: String, body: String) -> Result<(), String> {
    if !permission(app, false).await? {
        return Err("desktop notifications are not permitted".into());
    }
    platform::show(app, title, body).await
}

#[tauri::command]
pub async fn desktop_notification_permission_state(
    webview: Webview,
    app: AppHandle,
) -> Result<NotificationPermission, String> {
    require_main(&webview)?;
    platform::permission_state(&app).await
}

#[tauri::command]
pub async fn desktop_notification_permission(
    request: bool,
    webview: Webview,
    app: AppHandle,
) -> Result<bool, String> {
    require_main(&webview)?;
    permission(&app, request).await
}

#[tauri::command]
pub async fn desktop_notification_show(
    title: String,
    body: String,
    user_id: String,
    webview: Webview,
    app: AppHandle,
    active: State<'_, ActiveAccount>,
) -> Result<(), String> {
    require_main(&webview)?;
    if active.get().as_ref() != Some(&AccountId::parse(&user_id)?) {
        return Err("notification account does not match the active account".into());
    }
    show(&app, title, body).await
}

#[tauri::command]
pub fn desktop_notification_open_settings(webview: Webview) -> Result<(), String> {
    require_main(&webview)?;
    platform::open_settings()
}
