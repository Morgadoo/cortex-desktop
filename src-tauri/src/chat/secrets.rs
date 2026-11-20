//! The local profile's secrets in the OS keychain: one way to reach them.
//!
//! Provider keys, service tokens, the vault and the sync passphrase all live in the store of the
//! one local account ([`super::LOCAL_ACCOUNT`]); this module is where that account is named.
//! Integrations that only need a key import this, not the engine.

use crate::account::AccountId;
use crate::tools::auth::{keychain_secrets, Secrets};

/// The keychain store of the local profile.
pub fn local() -> Result<Secrets, String> {
    Ok(keychain_secrets(AccountId::parse(
        crate::chat::LOCAL_ACCOUNT,
    )?))
}

/// The API key for `vendor` from the keychain, if one is saved.
pub async fn api_key(vendor: &str) -> Result<Option<String>, String> {
    local()?
        .load(&crate::chat::providers::secret_name(vendor))
        .await
}
