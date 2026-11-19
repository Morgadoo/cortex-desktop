//! The privacy filter: personal data is swapped for placeholders before a request leaves for a
//! cloud provider, and swapped back in the reply.
//!
//! Emails, phone numbers, IBANs and card numbers in what the user typed are replaced with
//! `[EMAIL_1]`-style placeholders in the request only; the stored conversation keeps the real
//! text, and the model's reply is restored before the user sees it. Local providers (Ollama,
//! llama.cpp) never leave the machine, so nothing is filtered for them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use cortex_kernel::privacy::*;

use super::store::{chat_dir, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct PrivacySettings {
    pub enabled: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("privacy.json")
}

pub fn load(config_dir: &Path) -> PrivacySettings {
    crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &PrivacySettings) -> Result<(), String> {
    write_json(&path(config_dir), settings)
}
