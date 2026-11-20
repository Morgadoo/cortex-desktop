//! Bridge connection settings and the per-account file that keeps them.

use crate::account::AccountId;
use crate::reporting::config_failure;
use crate::support::error::StoreError;
use crate::support::settings_file::{self, Storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionSecurity {
    StartTls,
    Tls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtonMailConfig {
    pub imap_port: u16,
    pub smtp_port: u16,
    pub username: String,
    pub imap_security: ConnectionSecurity,
    pub smtp_security: ConnectionSecurity,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetupInput {
    pub(super) imap_port: u16,
    pub(super) smtp_port: u16,
    pub(super) username: String,
    #[serde(default)]
    pub(super) password: String,
    pub(super) imap_security: ConnectionSecurity,
    pub(super) smtp_security: ConnectionSecurity,
}

/// Where the Bridge connection settings are kept between launches.
pub trait ConfigFile: Send + Sync {
    fn load(&self) -> Result<Option<ProtonMailConfig>, String>;
    fn save(&self, config: &ProtonMailConfig) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

pub struct AccountConfigFile {
    pub(super) app_handle: AppHandle,
    pub(super) account: AccountId,
}

impl ConfigFile for AccountConfigFile {
    fn load(&self) -> Result<Option<ProtonMailConfig>, String> {
        load_config(&self.app_handle, &self.account)
    }

    fn save(&self, config: &ProtonMailConfig) -> Result<(), String> {
        save_config(&self.app_handle, &self.account, config)
    }

    fn delete(&self) -> Result<(), String> {
        let path = config_path(&self.app_handle, &self.account)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

pub(super) fn validate_config(config: &ProtonMailConfig) -> Result<(), String> {
    if config.username.is_empty() {
        return Err("Bridge username is required".into());
    }
    if config.imap_port == 0 || config.smtp_port == 0 {
        return Err("IMAP and SMTP ports must be between 1 and 65535".into());
    }
    Ok(())
}

pub(super) fn config_path(app_handle: &AppHandle, account: &AccountId) -> Result<PathBuf, String> {
    crate::settings::account_config_dir(app_handle, account).map(|dir| dir.join("proton_mail.json"))
}

pub(super) fn load_config(
    app_handle: &AppHandle,
    account: &AccountId,
) -> Result<Option<ProtonMailConfig>, String> {
    let path = config_path(app_handle, account)?;
    let Some(config): Option<ProtonMailConfig> = settings_file::read_json(&path, Storage::Plain)
        .inspect_err(|e| {
            if matches!(e, StoreError::Parse { .. }) {
                config_failure("Mail configuration load failed");
            }
        })?
    else {
        return Ok(None);
    };
    validate_config(&config).inspect_err(|_| config_failure("Mail configuration load failed"))?;
    Ok(Some(config))
}

pub(super) fn save_config(
    app_handle: &AppHandle,
    account: &AccountId,
    config: &ProtonMailConfig,
) -> Result<(), String> {
    let path = config_path(app_handle, account)?;
    settings_file::write_json(&path, Storage::Plain, config)
        .inspect_err(|e| {
            if matches!(e, StoreError::Encode { .. }) {
                config_failure("Mail configuration save failed");
            }
        })
        .map_err(String::from)
}
