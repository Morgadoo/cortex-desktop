//! Windows notification settings, delivery identity, and fail-open checks.

#[cfg(windows)]
use tauri::AppHandle;
#[cfg(windows)]
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
use super::NotificationPermission;

// Match tauri-plugin-notification's Windows delivery identity: binaries in
// target/debug and target/release use notify-rust's PowerShell fallback AUMID.
fn notification_app_id<'a>(exe_dir: &str, identifier: &'a str) -> &'a str {
    if exe_dir.ends_with(r"\target\debug") || exe_dir.ends_with(r"\target\release") {
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"
    } else {
        identifier
    }
}

#[cfg(windows)]
pub(super) async fn permission_state(app: &AppHandle) -> Result<NotificationPermission, String> {
    Ok(if permission_or_allow(read_permission(app)) {
        NotificationPermission::Granted
    } else {
        NotificationPermission::Denied
    })
}

fn permission_or_allow(result: Result<bool, String>) -> bool {
    match result {
        Ok(granted) => granted,
        Err(error) => {
            // Portable or unbundled apps may lack the registered shortcut needed
            // to query settings (e.g. HRESULT 0x80070490), but can still send.
            log::warn!(
                "could not check Windows notification permission; allowing delivery: {error}"
            );
            true
        }
    }
}

#[cfg(windows)]
fn read_permission(app: &AppHandle) -> Result<bool, String> {
    use ::windows::core::HSTRING;
    use ::windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};

    let exe = tauri::utils::platform::current_exe().map_err(|error| error.to_string())?;
    let exe_dir = exe
        .parent()
        .ok_or("notification executable has no parent directory")?
        .display()
        .to_string();
    let app_id = notification_app_id(&exe_dir, &app.config().identifier);
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
        .map_err(|error| format!("could not create Windows notification notifier: {error}"))?;
    let setting = notifier
        .Setting()
        .map_err(|error| format!("could not read Windows notification settings: {error}"))?;
    // Windows has no permission prompt. Both checking and requesting reflect the
    // OS setting; only the user or administrator can remove a notification block.
    match setting {
        NotificationSetting::Enabled => Ok(true),
        NotificationSetting::DisabledForApplication
        | NotificationSetting::DisabledForUser
        | NotificationSetting::DisabledByGroupPolicy
        | NotificationSetting::DisabledByManifest => Ok(false),
        _ => Err(format!("unknown Windows notification setting: {setting:?}")),
    }
}

#[cfg(windows)]
pub(super) async fn request_permission(app: &AppHandle) -> Result<bool, String> {
    app.notification()
        .request_permission()
        .map(|state| matches!(state, tauri::plugin::PermissionState::Granted))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(super) async fn show(app: &AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(super) fn open_settings() -> Result<(), String> {
    open::that_detached("ms-settings:notifications").map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{notification_app_id, permission_or_allow};

    #[test]
    fn unreadable_windows_settings_do_not_block_delivery() {
        for error in [
            "could not read Windows notification settings: 0x80070490",
            "could not create Windows notification notifier",
            "unknown Windows notification setting",
        ] {
            assert!(permission_or_allow(Err(error.into())));
        }
    }

    #[test]
    fn readable_windows_settings_preserve_enabled_and_blocked_results() {
        assert!(permission_or_allow(Ok(true)));
        assert!(!permission_or_allow(Ok(false)));
    }

    #[test]
    fn windows_permission_uses_installed_notification_identity() {
        let identifier = "io.github.morgadoo.cortex-desktop";
        for directory in [
            r"C:\Program Files\Cortex",
            r"C:\Users\user\AppData\Local\Cortex",
            r"C:\apps\debug",
            r"C:\repo\target\release\bundle",
        ] {
            assert_eq!(notification_app_id(directory, identifier), identifier);
        }
    }

    #[test]
    fn windows_permission_matches_uninstalled_delivery_fallback() {
        for directory in [r"C:\repo\target\debug", r"C:\repo\target\release"] {
            assert_eq!(
                notification_app_id(directory, "io.github.morgadoo.cortex-desktop"),
                r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"
            );
        }
    }
}
