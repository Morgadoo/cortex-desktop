//! The privacy filter: personal data is swapped for placeholders before a request leaves for a
//! cloud provider, and swapped back in the reply.
//!
//! Emails, phone numbers, IBANs and card numbers in what the user typed are replaced with
//! `[EMAIL_1]`-style placeholders in the request only; the stored conversation keeps the real
//! text, and the model's reply is restored before the user sees it. Local providers (Ollama,
//! llama.cpp) never leave the machine, so nothing is filtered for them.

use crate::chat::error::ChatError;
use crate::support::settings_file::SettingsFile;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use cortex_kernel::privacy::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct PrivacySettings {
    pub enabled: bool,
}

const PRIVACY_FILE: SettingsFile<PrivacySettings> = SettingsFile::sealed("chat/privacy.json");

pub fn load(config_dir: &Path) -> PrivacySettings {
    PRIVACY_FILE.load(config_dir)
}

pub fn save(config_dir: &Path, settings: &PrivacySettings) -> Result<(), ChatError> {
    PRIVACY_FILE
        .save(config_dir, settings)
        .map_err(String::from)
        .map_err(ChatError::Invalid)
}
