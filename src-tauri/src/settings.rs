pub mod commands;
pub mod window;

use super::reporting::config_failure;
use crate::account::AccountId;
use crate::support::settings_file::{self, Storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub const KEYCHAIN_SERVICE: &str = crate::rename_migration::ID;
/// Lumo Desktop's keychain service; secrets still there move over when first read.
#[cfg(not(windows))]
pub const LEGACY_KEYCHAIN_SERVICE: &str = crate::rename_migration::LEGACY_ID;

/// The IPC shape the settings page reads and writes. Persisted as [`DeviceSettings`] plus
/// [`AccountSettings`] in separate files.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AppSettings {
    pub search_shortcut: String,
    #[serde(default = "default_true")]
    pub search_shortcut_enabled: bool,
    pub launch_at_startup: bool,
    #[serde(default)]
    pub filesystem_allowed_roots: Vec<String>,
    #[serde(default)]
    pub filesystem_write_enabled: bool,
    #[serde(default)]
    pub bash_enabled: bool,
    #[serde(default = "default_true")]
    pub settings_hotkey_enabled: bool,
}

/// Global rather than per-account: the shortcut is registered before any account is known, and
/// launch-at-startup is a single OS registration.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceSettings {
    pub search_shortcut: String,
    #[serde(default = "default_true")]
    pub search_shortcut_enabled: bool,
    pub launch_at_startup: bool,
    #[serde(default = "default_true")]
    pub settings_hotkey_enabled: bool,
}

/// Capability grants, stored per account. A grant made under one account must not apply under
/// another.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AccountSettings {
    #[serde(default)]
    pub filesystem_allowed_roots: Vec<String>,
    #[serde(default)]
    pub filesystem_write_enabled: bool,
    #[serde(default)]
    pub bash_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            search_shortcut: "Ctrl+L".to_string(),
            search_shortcut_enabled: true,
            launch_at_startup: false,
            settings_hotkey_enabled: true,
        }
    }
}

impl AppSettings {
    pub fn merge(device: DeviceSettings, account: AccountSettings) -> Self {
        Self {
            search_shortcut: device.search_shortcut,
            search_shortcut_enabled: device.search_shortcut_enabled,
            launch_at_startup: device.launch_at_startup,
            settings_hotkey_enabled: device.settings_hotkey_enabled,
            filesystem_allowed_roots: account.filesystem_allowed_roots,
            filesystem_write_enabled: account.filesystem_write_enabled,
            bash_enabled: account.bash_enabled,
        }
    }

    pub fn split(&self) -> (DeviceSettings, AccountSettings) {
        (
            DeviceSettings {
                search_shortcut: self.search_shortcut.clone(),
                search_shortcut_enabled: self.search_shortcut_enabled,
                launch_at_startup: self.launch_at_startup,
                settings_hotkey_enabled: self.settings_hotkey_enabled,
            },
            AccountSettings {
                filesystem_allowed_roots: self.filesystem_allowed_roots.clone(),
                filesystem_write_enabled: self.filesystem_write_enabled,
                bash_enabled: self.bash_enabled,
            },
        )
    }
}

/// The app's own settings stay plain JSON: they are read before encryption at rest is unlocked.
fn read_json<T: Default + for<'de> Deserialize<'de>>(path: PathBuf) -> Result<T, String> {
    settings_file::read_json(&path, Storage::Plain)
        .map(Option::unwrap_or_default)
        .inspect_err(|_| config_failure("Settings load failed"))
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), String> {
    settings_file::write_json(&path, Storage::Plain, value)
        .inspect_err(|_| config_failure("Settings save failed"))
}

pub fn get_settings_path(app_handle: &AppHandle) -> Result<PathBuf, String> {
    get_config_dir(app_handle).map(|dir| dir.join("settings.json"))
}

fn account_settings_path(app_handle: &AppHandle, account: &AccountId) -> Result<PathBuf, String> {
    account_config_dir(app_handle, account).map(|dir| dir.join("settings.json"))
}

pub fn load_device_settings(app_handle: &AppHandle) -> Result<DeviceSettings, String> {
    read_json(get_settings_path(app_handle)?)
}

pub fn save_device_settings(
    app_handle: &AppHandle,
    settings: &DeviceSettings,
) -> Result<(), String> {
    write_json(get_settings_path(app_handle)?, settings)
}

pub fn load_account_settings(
    app_handle: &AppHandle,
    account: &AccountId,
) -> Result<AccountSettings, String> {
    read_json(account_settings_path(app_handle, account)?)
}

pub fn save_account_settings(
    app_handle: &AppHandle,
    account: &AccountId,
    settings: &AccountSettings,
) -> Result<(), String> {
    write_json(account_settings_path(app_handle, account)?, settings)
}

pub fn get_config_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map_err(|e| format!("Could not find config directory: {e}"))
}

/// Uses the account id as a path segment, which [`AccountId`] validation makes safe.
pub fn account_config_dir(app_handle: &AppHandle, account: &AccountId) -> Result<PathBuf, String> {
    get_config_dir(app_handle).map(|dir| dir.join("accounts").join(account.as_str()))
}

#[cfg(test)]
mod tests {
    use super::{AccountSettings, AppSettings, DeviceSettings};

    /// A field dropped by either direction would silently reset a capability grant.
    #[test]
    fn splitting_and_merging_preserves_every_field() {
        let merged = AppSettings::merge(
            DeviceSettings {
                search_shortcut: "Cmd+K".to_string(),
                search_shortcut_enabled: false,
                launch_at_startup: true,
                settings_hotkey_enabled: false,
            },
            AccountSettings {
                filesystem_allowed_roots: vec!["/tmp/work".to_string()],
                filesystem_write_enabled: true,
                bash_enabled: true,
            },
        );

        let (device, account) = merged.split();

        assert_eq!(device.search_shortcut, "Cmd+K");
        assert!(!device.search_shortcut_enabled);
        assert!(device.launch_at_startup);
        assert!(!device.settings_hotkey_enabled);
        assert_eq!(
            account.filesystem_allowed_roots,
            vec!["/tmp/work".to_string()]
        );
        assert!(account.filesystem_write_enabled);
        assert!(account.bash_enabled);
    }
}
