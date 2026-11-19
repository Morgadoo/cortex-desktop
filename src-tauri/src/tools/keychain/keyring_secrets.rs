use crate::reporting::credential_failure;
use crate::settings::{KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE};
use keyring::Entry;

fn describe_keychain_error(action: &str, err: &keyring::Error) -> String {
    if matches!(err, keyring::Error::BadEncoding(_)) {
        credential_failure("Credential decoding failed");
    }
    match err {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => format!(
            "secure storage is unavailable (it may be locked or not running); could not {action}: {err}"
        ),
        _ => format!("could not {action}: {err}"),
    }
}

pub fn store_secret_blocking(account: &str, secret: &str) -> Result<(), String> {
    let entry = Entry::new(KEYCHAIN_SERVICE, account)
        .map_err(|e| describe_keychain_error("open secure storage", &e))?;
    entry
        .set_password(secret)
        .map_err(|e| describe_keychain_error("store secret", &e))
}

pub fn load_secret_blocking(account: &str) -> Result<Option<String>, String> {
    let entry = Entry::new(KEYCHAIN_SERVICE, account)
        .map_err(|e| describe_keychain_error("open secure storage", &e))?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => migrate_legacy_secret(&entry, account),
        Err(e) => Err(describe_keychain_error("load secret", &e)),
    }
}

/// A secret Lumo Desktop stored under its own keychain service moves to the current one the first
/// time it is read. The old entry is removed only once the new one is written.
fn migrate_legacy_secret(entry: &Entry, account: &str) -> Result<Option<String>, String> {
    let Ok(legacy) = Entry::new(LEGACY_KEYCHAIN_SERVICE, account) else {
        return Ok(None);
    };
    let value = match legacy.get_password() {
        Ok(value) => value,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(describe_keychain_error("load secret", &e)),
    };
    entry
        .set_password(&value)
        .map_err(|e| describe_keychain_error("store secret", &e))?;
    let _ = legacy.delete_credential();
    Ok(Some(value))
}

pub fn delete_secret_blocking(account: &str) -> Result<(), String> {
    // The legacy copy goes too, or the next read would bring the deleted secret back.
    for service in [KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE] {
        let entry = Entry::new(service, account)
            .map_err(|e| describe_keychain_error("open secure storage", &e))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(describe_keychain_error("delete secret", &e)),
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn delete_account_secrets_blocking(account: &crate::account::AccountId) -> Result<(), String> {
    delete_service_account_secrets(KEYCHAIN_SERVICE, account)?;
    delete_service_account_secrets(LEGACY_KEYCHAIN_SERVICE, account)
}

#[cfg(target_os = "macos")]
fn delete_service_account_secrets(
    service: &str,
    account: &crate::account::AccountId,
) -> Result<(), String> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
    let items = match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .load_attributes(true)
        .limit(Limit::All)
        .search()
    {
        Ok(items) => items,
        Err(e) if e.code() == -25300 => return Ok(()), // errSecItemNotFound
        Err(e) => return Err(format!("could not find account credentials: {e}")),
    };
    for item in items {
        let attributes = item
            .simplify_dict()
            .ok_or("could not read credential attributes")?;
        // The macOS Keychain account attribute is named `acct`.
        let key = attributes
            .get("acct")
            .ok_or("credential has no account attribute")?;
        if !crate::tools::auth::secret_belongs_to_account(key, account) {
            continue;
        }
        match ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(service)
            .account(key)
            .case_insensitive(Some(false))
            .limit(Limit::All)
            .delete()
        {
            Ok(()) => {}
            Err(e) if e.code() == -25300 => {}
            Err(e) => return Err(format!("could not delete account credentials: {e}")),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn delete_account_secrets_blocking(account: &crate::account::AccountId) -> Result<(), String> {
    use dbus_secret_service::{EncryptionType, SecretService};
    let delete = || -> Result<(), dbus_secret_service::Error> {
        let service = SecretService::connect(EncryptionType::Dh)?;
        for name in [KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE] {
            let found =
                service.search_items(std::collections::HashMap::from([("service", name)]))?;
            for item in found.unlocked.iter().chain(found.locked.iter()) {
                let attributes = item.get_attributes()?;
                if attributes
                    .get("username")
                    .is_some_and(|key| crate::tools::auth::secret_belongs_to_account(key, account))
                {
                    item.unlock()?;
                    item.delete()?;
                }
            }
        }
        Ok(())
    };
    delete().map_err(|e| format!("could not delete account credentials: {e}"))
}

const PROBE_ACCOUNT: &str = "__storage_probe__";

/// Reads a key that is never written, so a "no such entry" answer is the *positive*
/// signal: only a reachable provider can search and report the key absent.
fn probe_says_available(result: Result<String, keyring::Error>) -> bool {
    !matches!(
        result,
        Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_))
    )
}

pub fn secure_storage_available_blocking() -> bool {
    match Entry::new(KEYCHAIN_SERVICE, PROBE_ACCOUNT) {
        Ok(entry) => probe_says_available(entry.get_password()),
        Err(e) => probe_says_available(Err(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::probe_says_available;
    use keyring::Error;

    fn platform_error() -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(std::io::Error::other("no session bus"))
    }

    #[test]
    fn a_missing_probe_key_means_storage_is_reachable() {
        assert!(probe_says_available(Err(Error::NoEntry)));
    }

    #[test]
    fn unreachable_storage_is_reported_unavailable() {
        assert!(!probe_says_available(Err(Error::NoStorageAccess(
            platform_error()
        ))));
        assert!(!probe_says_available(Err(Error::PlatformFailure(
            platform_error()
        ))));
    }

    #[test]
    fn a_leftover_probe_key_still_means_reachable() {
        assert!(probe_says_available(Ok("leftover".to_string())));
    }

    #[test]
    fn errors_unrelated_to_reachability_do_not_condemn_storage() {
        assert!(probe_says_available(Err(Error::TooLong(
            "account".to_string(),
            128
        ))));
        assert!(probe_says_available(Err(Error::BadEncoding(vec![0xff]))));
    }
}
