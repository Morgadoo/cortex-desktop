//! The local profile's secrets in the OS keychain: one way to reach them.
//!
//! Provider keys, service tokens, the vault and the sync passphrase all live in the store of the
//! one local account ([`super::LOCAL_ACCOUNT`]); this module is where that account is named.
//! Provider keys are looked up by `providers::api_key`.

use crate::account::AccountId;
use crate::support::error::StoreError;
use crate::tools::auth::{keychain_secrets, Secrets};

/// The keychain store of the local profile.
pub fn local() -> Result<Secrets, StoreError> {
    let account = AccountId::parse(crate::chat::LOCAL_ACCOUNT).map_err(StoreError::Keychain)?;
    Ok(keychain_secrets(account))
}
