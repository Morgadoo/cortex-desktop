//! Applies pending resets before background tasks can write configuration.
//!
//! Removes only the requested account's configuration; shared settings and webview storage remain.

use super::{
    account::{AccountId, ActiveAccount},
    settings::get_config_dir,
    tools::auth::delete_account_secrets_blocking,
};
use std::{fs, io::ErrorKind, path::Path};
use tauri::{AppHandle, Manager, Webview};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const RESET_MARKER: &str = "reset-pending";

#[tauri::command]
pub async fn reset_local_configuration(
    app_handle: AppHandle,
    webview: Webview,
) -> Result<(), String> {
    if webview.label() != crate::settings::window::LABEL {
        return Err("Local configuration can only be reset from Settings".into());
    }
    let account = app_handle
        .state::<ActiveAccount>()
        .get()
        .ok_or("Sign in to reset account configuration")?;
    let handle = app_handle.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        handle.dialog()
            .message("This resets tool permissions and removes connector configurations and saved credentials for your current account on this device. You’ll need to reconnect your connectors.\n\nCortex will restart.")
            .title("Reset account configuration?")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Reset and restart".into(), "Cancel".into(),
            ))
            .blocking_show()
    }).await.map_err(|e| format!("Could not show reset warning: {e}"))?;
    if !accepted {
        return Ok(());
    }
    if app_handle.state::<ActiveAccount>().get().as_ref() != Some(&account) {
        return Err("The active account changed. Please try resetting again.".into());
    }
    let dir = get_config_dir(&app_handle)?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not prepare reset: {e}"))?;
    fs::write(dir.join(RESET_MARKER), account.as_str())
        .map_err(|e| format!("Could not schedule reset: {e}"))?;
    app_handle.restart();
}

pub fn apply_pending_reset(app_handle: &AppHandle) -> Result<(), String> {
    reset_if_pending(
        &get_config_dir(app_handle)?,
        delete_account_secrets_blocking,
    )
}

pub fn cancel_pending_reset(app_handle: &AppHandle) -> Result<(), String> {
    clear_reset_marker(&get_config_dir(app_handle)?)
}

fn clear_reset_marker(dir: &Path) -> Result<(), String> {
    match fs::remove_file(dir.join(RESET_MARKER)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Could not remove pending reset request: {e}")),
    }
}

fn reset_if_pending(
    dir: &Path,
    delete_secrets: impl FnOnce(&AccountId) -> Result<(), String>,
) -> Result<(), String> {
    let requested_account = match fs::read_to_string(dir.join(RESET_MARKER)) {
        Ok(account) => AccountId::parse(&account)
            .map_err(|e| format!("Invalid account in reset request: {e}"))?,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Could not read reset request: {e}")),
    };
    let accounts_dir = dir.join("accounts");
    match fs::symlink_metadata(&accounts_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("Cannot reset configuration through a linked accounts directory".into())
        }
        Err(e) if e.kind() != ErrorKind::NotFound => {
            return Err(format!("Could not inspect accounts directory: {e}"))
        }
        _ => {}
    }
    // Preserve configuration for retry if credential deletion fails.
    delete_secrets(&requested_account)?;
    remove_config_path(&accounts_dir.join(requested_account.as_str()))?;
    // Retain the marker until cleanup succeeds so interrupted resets are retried.
    clear_reset_marker(dir)
}

fn remove_config_path(path: &Path) -> Result<(), String> {
    // Inspect the link itself to avoid following it outside the configuration directory.
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Could not inspect {}: {e}", path.display())),
    };
    let result = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| format!("Could not remove {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "first-account";

    fn request_reset(dir: &Path) {
        fs::write(dir.join(RESET_MARKER), ACCOUNT).unwrap();
    }

    fn account_config(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("accounts").join(ACCOUNT).join("settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "saved settings").unwrap();
        path
    }

    #[test]
    fn ordinary_startup_leaves_configuration_and_secrets_alone() {
        let dir = tempfile::tempdir().unwrap();
        let settings = account_config(dir.path());
        reset_if_pending(dir.path(), |_| {
            panic!("must not touch secrets without a request")
        })
        .unwrap();
        assert_eq!(fs::read_to_string(settings).unwrap(), "saved settings");
    }

    #[test]
    fn reset_targets_recorded_account_and_preserves_other_accounts_device_and_webview_data() {
        let dir = tempfile::tempdir().unwrap();
        request_reset(dir.path());
        let settings = account_config(dir.path());
        fs::write(&settings, "{invalid json").unwrap();
        let cache = settings.parent().unwrap().join("mcp_tools/server.json");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(cache, "corrupt").unwrap();

        let preserved = [
            "settings.json",
            "mcp_servers.json",
            "accounts/second-account/settings.json",
            "WebKit/WebsiteData/LocalStorage/session",
            "Cookies",
            "unrelated-file",
        ];
        for name in preserved {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name.as_bytes()).unwrap();
        }
        let mut secrets_deleted = false;
        reset_if_pending(dir.path(), |account| {
            assert_eq!(account.as_str(), ACCOUNT);
            assert!(settings.exists());
            secrets_deleted = true;
            Ok(())
        })
        .unwrap();

        assert!(secrets_deleted);
        assert!(!settings.parent().unwrap().exists());
        assert!(!dir.path().join(RESET_MARKER).exists());
        for name in preserved {
            assert_eq!(fs::read(dir.path().join(name)).unwrap(), name.as_bytes());
        }
    }

    #[test]
    fn invalid_or_legacy_reset_requests_cannot_delete_configuration_or_secrets() {
        for request in ["", "../other-account", "first-account/../../other-account"] {
            let dir = tempfile::tempdir().unwrap();
            let settings = account_config(dir.path());
            fs::write(dir.path().join(RESET_MARKER), request).unwrap();
            assert!(reset_if_pending(dir.path(), |_| panic!(
                "invalid requests must not touch secrets"
            ))
            .is_err());
            assert!(settings.exists());
            clear_reset_marker(dir.path()).unwrap();
        }
    }

    #[test]
    fn denied_secret_cleanup_preserves_config_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        request_reset(dir.path());
        let settings = account_config(dir.path());

        assert_eq!(
            reset_if_pending(dir.path(), |_| Err("storage locked".into())),
            Err("storage locked".into())
        );
        assert!(dir.path().join(RESET_MARKER).exists());
        assert!(settings.exists());

        reset_if_pending(dir.path(), |_| Ok(())).unwrap();
        assert!(!settings.exists());
        assert!(!dir.path().join(RESET_MARKER).exists());
    }

    #[test]
    fn cancelling_a_failed_reset_allows_startup_without_accessing_secrets() {
        let dir = tempfile::tempdir().unwrap();
        request_reset(dir.path());
        let settings = account_config(dir.path());

        assert!(reset_if_pending(dir.path(), |_| Err("storage unavailable".into())).is_err());
        clear_reset_marker(dir.path()).unwrap();
        assert!(!dir.path().join(RESET_MARKER).exists());
        reset_if_pending(dir.path(), |_| {
            panic!("cancelled reset must not access secrets")
        })
        .unwrap();
        assert_eq!(fs::read_to_string(settings).unwrap(), "saved settings");
    }

    #[test]
    fn missing_configs_are_safe_and_completed_reset_is_not_repeated() {
        let dir = tempfile::tempdir().unwrap();
        request_reset(dir.path());
        reset_if_pending(dir.path(), |_| Ok(())).unwrap();
        reset_if_pending(dir.path(), |_| panic!("completed reset must not run again")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn configuration_symlinks_do_not_delete_files_outside_the_app() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep.json"), "keep").unwrap();
        fs::create_dir(dir.path().join("accounts")).unwrap();
        symlink(outside.path(), dir.path().join("accounts").join(ACCOUNT)).unwrap();
        request_reset(dir.path());

        reset_if_pending(dir.path(), |_| Ok(())).unwrap();

        assert_eq!(
            fs::read_to_string(outside.path().join("keep.json")).unwrap(),
            "keep"
        );
        assert!(fs::symlink_metadata(dir.path().join("accounts").join(ACCOUNT)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_accounts_parent_cannot_redirect_reset_outside_the_app() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join(ACCOUNT)).unwrap();
        symlink(outside.path(), dir.path().join("accounts")).unwrap();
        request_reset(dir.path());

        assert!(reset_if_pending(dir.path(), |_| panic!(
            "linked accounts must not touch secrets"
        ))
        .is_err());
        assert!(outside.path().join(ACCOUNT).exists());
        assert!(dir.path().join(RESET_MARKER).exists());
    }
}
