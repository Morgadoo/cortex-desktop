use crate::account::AccountId;
use crate::reporting::config_failure;
use crate::support::error::StoreError;
use crate::support::settings_file::{self, Storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

const CONFIG_FILE: &str = "luminance.json";
const VENDOR_DOMAIN: &str = "luminance.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuminanceAuth {
    ClientCredentials,
    AccessToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuminanceConfig {
    /// Origin of the customer's instance, e.g. `https://acme.app.luminance.com`.
    pub instance_url: String,
    pub auth: LuminanceAuth,
}

/// Where the instance settings are kept between launches.
pub trait ConfigFile: Send + Sync {
    fn load(&self) -> Result<Option<LuminanceConfig>, String>;
    fn save(&self, config: &LuminanceConfig) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

pub struct AccountConfigFile {
    pub(super) app_handle: AppHandle,
    pub(super) account: AccountId,
}

impl ConfigFile for AccountConfigFile {
    fn load(&self) -> Result<Option<LuminanceConfig>, String> {
        let path = config_path(&self.app_handle, &self.account)?;
        let Some(config): Option<LuminanceConfig> = settings_file::read_json(&path, Storage::Plain)
            .inspect_err(|e| {
                if matches!(e, StoreError::Parse { .. }) {
                    config_failure("Luminance configuration load failed");
                }
            })?
        else {
            return Ok(None);
        };
        instance_origin(&config.instance_url)
            .inspect_err(|_| config_failure("Luminance configuration load failed"))?;
        Ok(Some(config))
    }

    fn save(&self, config: &LuminanceConfig) -> Result<(), String> {
        let path = config_path(&self.app_handle, &self.account)?;
        settings_file::write_json(&path, Storage::Plain, config)
            .inspect_err(|e| {
                if matches!(e, StoreError::Encode { .. }) {
                    config_failure("Luminance configuration save failed");
                }
            })
            .map_err(String::from)
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

pub(super) fn config_path(app_handle: &AppHandle, account: &AccountId) -> Result<PathBuf, String> {
    crate::settings::account_config_dir(app_handle, account).map(|dir| dir.join(CONFIG_FILE))
}

/// Normalises what the user typed (a moniker, a host, or a URL) to the instance origin.
///
/// Credentials are only ever sent to Luminance's own hosts.
pub(super) fn instance_origin(input: &str) -> Result<String, String> {
    let raw = input.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("enter your Luminance instance".into());
    }
    let with_host = if raw.contains("://") || raw.contains('.') {
        raw.to_string()
    } else {
        format!("{raw}.app.{VENDOR_DOMAIN}")
    };
    let with_scheme = if with_host.contains("://") {
        with_host
    } else {
        format!("https://{with_host}")
    };
    let url = url::Url::parse(&with_scheme)
        .map_err(|_| format!("'{input}' is not a valid Luminance instance"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if url.scheme() != "https" {
        return Err("the Luminance instance must use https".into());
    }
    if !host.ends_with(&format!(".{VENDOR_DOMAIN}")) {
        return Err(format!(
            "'{host}' is not a Luminance host; use your instance moniker or its https://<moniker>.app.{VENDOR_DOMAIN} address"
        ));
    }
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("the Luminance instance address must not include a port or credentials".into());
    }
    Ok(format!("https://{host}"))
}

pub(super) struct NoConfigFile;

impl ConfigFile for NoConfigFile {
    fn load(&self) -> Result<Option<LuminanceConfig>, String> {
        Ok(None)
    }

    fn save(&self, _config: &LuminanceConfig) -> Result<(), String> {
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moniker_host_or_url_all_resolve_to_the_instance_origin() {
        for input in [
            "acme",
            "acme.app.luminance.com",
            "https://acme.app.luminance.com",
            "https://ACME.app.luminance.com/api2/",
            "  acme  ",
        ] {
            assert_eq!(
                instance_origin(input).unwrap(),
                "https://acme.app.luminance.com",
                "{input}"
            );
        }
    }

    #[test]
    fn credentials_are_only_sent_to_luminance_over_https() {
        for input in [
            "",
            "http://acme.app.luminance.com",
            "https://luminance.com.evil.example",
            "https://evilluminance.com",
            "https://acme.app.luminance.com:8443",
            "https://user:pw@acme.app.luminance.com",
        ] {
            assert!(
                instance_origin(input).is_err(),
                "{input} should be rejected"
            );
        }
    }
}
