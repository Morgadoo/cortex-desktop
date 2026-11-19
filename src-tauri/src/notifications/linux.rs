//! Best-effort permission checks for native GNOME and KDE notification settings.
//! These desktops do not have a notification permission prompt. Never change the
//! user's settings, or prevent delivery when their settings cannot be read.

use std::process::Output;
use std::time::Duration;

#[cfg(target_os = "linux")]
use tauri::AppHandle;
#[cfg(target_os = "linux")]
use tauri_plugin_notification::NotificationExt;

#[cfg(target_os = "linux")]
use super::NotificationPermission;

#[cfg(target_os = "linux")]
fn desktop_entry(config: &tauri::Config) -> &str {
    // Tauri's DEB/RPM bundler names the desktop file after productName, not
    // the bundle identifier. The Linux override deliberately uses cortex.desktop.
    config.product_name.as_deref().unwrap_or("Cortex")
}

#[cfg(target_os = "linux")]
fn icon_name(config: &tauri::Config) -> &str {
    // Hicolor icons are named after mainBinaryName (cortex-desktop), independently
    // of productName (cortex).
    config
        .main_binary_name
        .as_deref()
        .unwrap_or_else(|| desktop_entry(config))
}

#[cfg(target_os = "linux")]
pub(super) async fn permission_state(app: &AppHandle) -> Result<NotificationPermission, String> {
    let desktop = Desktop::from_session(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    let granted = permission(desktop, desktop_entry(app.config())).await;
    Ok(if granted {
        NotificationPermission::Granted
    } else {
        NotificationPermission::Denied
    })
}

#[cfg(target_os = "linux")]
pub(super) async fn request_permission(app: &AppHandle) -> Result<bool, String> {
    app.notification()
        .request_permission()
        .map(|state| matches!(state, tauri::plugin::PermissionState::Granted))
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
pub(super) async fn show(app: &AppHandle, title: String, body: String) -> Result<(), String> {
    let desktop_entry = desktop_entry(app.config()).to_owned();
    let icon = icon_name(app.config()).to_owned();
    // The plugin doesn't set desktop-entry on Linux. Set it explicitly so
    // notification delivery uses the same application settings we checked.
    tokio::task::spawn_blocking(move || {
        notify_rust::Notification::new()
            .appname(&desktop_entry)
            .summary(&title)
            .body(&body)
            .icon(&icon)
            .hint(notify_rust::Hint::DesktopEntry(desktop_entry.clone()))
            .show()
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(target_os = "linux")]
pub(super) fn open_settings() -> Result<(), String> {
    let desktop = Desktop::from_session(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    let (program, page) = desktop.settings_command()?;
    let mut child = std::process::Command::new(program)
        .arg(page)
        .spawn()
        .map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Desktop {
    Gnome,
    Kde,
    Other,
}

impl Desktop {
    fn from_session(session: &str) -> Self {
        for desktop in session.split(':') {
            match desktop.to_ascii_lowercase().as_str() {
                "gnome" => return Self::Gnome,
                "kde" | "plasma" => return Self::Kde,
                _ => {}
            }
        }
        Self::Other
    }

    fn settings_command(self) -> Result<(&'static str, &'static str), String> {
        match self {
            Self::Gnome => Ok(("gnome-control-center", "notifications")),
            Self::Kde => Ok(("systemsettings", "kcm_notifications")),
            Self::Other => Err(
                "Open your desktop's notification settings manually; automatic settings access is supported on GNOME and KDE"
                    .into(),
            ),
        }
    }
}

async fn permission(desktop: Desktop, desktop_entry: &str) -> bool {
    let result = match desktop {
        Desktop::Gnome => {
            let schema = format!(
                "org.gnome.desktop.notifications.application:/org/gnome/desktop/notifications/application/{}/",
                gnome_application_id(desktop_entry)
            );
            query_bool("gsettings", &["get", &schema, "enable"]).await
        }
        Desktop::Kde => {
            // Popups and notification history are independent in Plasma. Do not
            // discard history-only notifications or treat Do Not Disturb as denial.
            let (popups, history) = tokio::join!(
                kde_setting(desktop_entry, "ShowPopups"),
                kde_setting(desktop_entry, "ShowInHistory"),
            );
            combine_kde_settings(popups, history)
        }
        Desktop::Other => return true,
    };
    permission_or_allow(result)
}

fn permission_or_allow(result: Result<bool, String>) -> bool {
    result.unwrap_or_else(|error| {
        log::warn!("could not check Linux notification permission; allowing delivery: {error}");
        true
    })
}

fn combine_kde_settings(
    popups: Result<bool, String>,
    history: Result<bool, String>,
) -> Result<bool, String> {
    Ok(popups? || history?)
}

fn gnome_application_id(desktop_entry: &str) -> String {
    // Match GNOME Shell's NotificationApplicationPolicy._canonicalizeId().
    let mut canonical = String::new();
    for character in desktop_entry.to_lowercase().chars() {
        let character = if character.is_ascii_alphanumeric() {
            character
        } else {
            '-'
        };
        if character != '-' || !canonical.ends_with('-') {
            canonical.push(character);
        }
    }
    canonical
}

async fn kde_setting(desktop_entry: &str, key: &str) -> Result<bool, String> {
    // Use KConfig's reader instead of parsing plasmanotifyrc ourselves: it also
    // handles inherited system defaults and the absence of an application group.
    let arguments = [
        "--file",
        "plasmanotifyrc",
        "--group",
        "Applications",
        "--group",
        desktop_entry,
        "--key",
        key,
        "--default",
        "true",
    ];
    match query_bool("kreadconfig6", &arguments).await {
        Ok(value) => Ok(value),
        Err(_) => query_bool("kreadconfig5", &arguments).await,
    }
}

async fn query_bool(program: &str, arguments: &[&str]) -> Result<bool, String> {
    // A hung settings service must not indefinitely delay notification delivery.
    let output = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::process::Command::new(program)
            .args(arguments)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{program} timed out"))?
    .map_err(|error| format!("{program}: {error}"))?;
    parse_bool_output(program, output)
}

fn parse_bool_output(program: &str, output: Output) -> Result<bool, String> {
    if !output.status.success() {
        return Err(format!("{program} exited with {}", output.status));
    }
    match std::str::from_utf8(&output.stdout).map(str::trim) {
        Ok("true") => Ok(true),
        Ok("false") => Ok(false),
        _ => Err(format!("{program} returned an invalid boolean")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_supported_desktops_in_session_lists() {
        assert_eq!(Desktop::from_session("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(Desktop::from_session("GNOME-Classic:GNOME"), Desktop::Gnome);
        assert_eq!(Desktop::from_session("KDE"), Desktop::Kde);
        assert_eq!(Desktop::from_session("plasma"), Desktop::Kde);
        for session in ["", "X-Cinnamon", "XFCE", "MATE", "LXQt", "notgnome"] {
            assert_eq!(Desktop::from_session(session), Desktop::Other);
            assert!(Desktop::from_session(session).settings_command().is_err());
        }
    }

    #[test]
    fn matches_gnome_settings_paths_for_desktop_file_ids() {
        assert_eq!(gnome_application_id("Cortex"), "cortex");
        assert_eq!(
            gnome_application_id("io.github.morgadoo.cortex-desktop"),
            "io-github-morgadoo-cortex-desktop"
        );
        assert_eq!(
            gnome_application_id("Cortex... Test--App"),
            "cortex-test-app"
        );
    }

    #[test]
    fn kde_blocks_only_when_both_popups_and_history_are_disabled() {
        assert!(!combine_kde_settings(Ok(false), Ok(false)).unwrap());
        assert!(combine_kde_settings(Ok(false), Ok(true)).unwrap());
        assert!(combine_kde_settings(Ok(true), Ok(false)).unwrap());
        assert!(combine_kde_settings(Ok(true), Ok(true)).unwrap());
        assert!(permission_or_allow(combine_kde_settings(
            Err("unreadable".into()),
            Ok(false)
        )));
        assert!(permission_or_allow(combine_kde_settings(
            Ok(false),
            Err("unreadable".into())
        )));
    }

    #[tokio::test]
    async fn unknown_desktops_do_not_require_settings_tools() {
        assert!(permission(Desktop::Other, "Cortex").await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_results_distinguish_denial_from_unreadable_settings() {
        assert!(query_bool("sh", &["-c", "printf 'true\\n'"]).await.unwrap());
        assert!(!query_bool("sh", &["-c", "printf 'false\\n'"])
            .await
            .unwrap());
        for script in ["printf false; exit 1", "printf unexpected", "printf ''"] {
            assert!(permission_or_allow(query_bool("sh", &["-c", script]).await));
        }
        assert!(permission_or_allow(
            query_bool("/nonexistent/cortex-settings-tool", &[]).await
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hung_settings_tools_fail_open() {
        let result = query_bool("sh", &["-c", "exec sleep 10"]).await;
        assert!(result.as_ref().unwrap_err().contains("timed out"));
        assert!(permission_or_allow(result));
    }
}
