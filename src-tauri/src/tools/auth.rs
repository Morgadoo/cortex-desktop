use super::keychain::{backend, pending_deletes};
use crate::account::{AccountId, ActiveAccount};
use async_trait::async_trait;
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use tokio::task::spawn_blocking;

// The credential contract (statuses, auth mechanisms, the secret store trait) is cortex-tools';
// the keychain behind it is here.
pub use cortex_tools::credentials::*;

#[cfg(not(target_os = "macos"))]
fn describe_join_error(err: tokio::task::JoinError) -> String {
    format!("secure storage task failed: {err}")
}

async fn storage_task<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    #[cfg(target_os = "macos")]
    {
        super::keychain::macos_queue::run(operation).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        spawn_blocking(operation)
            .await
            .map_err(describe_join_error)?
    }
}

/// Every key is composed here, so no caller can reach an entry outside its account's namespace.
pub struct KeychainSecrets {
    account: AccountId,
}

impl KeychainSecrets {
    pub fn new(account: AccountId) -> Self {
        Self { account }
    }

    fn keychain_account(&self, name: &str) -> String {
        format!("connector:{}:{name}", self.account.as_str())
    }
}

/// Where deletes the keychain couldn't perform are recorded; set once at startup.
pub fn set_pending_deletes_dir(config_dir: std::path::PathBuf) {
    pending_deletes::set_root(config_dir);
}

/// Deletes that fail are recorded in [`pending_deletes`] rather than reported, and a recorded
/// entry is never loaded: it is deleted first, and the load fails while that is impossible.
#[async_trait]
impl SecretStore for KeychainSecrets {
    async fn store(&self, name: &str, secret: &str) -> Result<(), String> {
        let account = self.keychain_account(name);
        let secret = secret.to_string();
        storage_task(move || {
            pending_deletes::store(&account, || {
                backend::store_secret_blocking(&account, &secret)
            })
        })
        .await
    }

    async fn load(&self, name: &str) -> Result<Option<String>, String> {
        let account = self.keychain_account(name);
        storage_task(move || {
            pending_deletes::load(
                &account,
                || backend::delete_secret_blocking(&account),
                || backend::load_secret_blocking(&account),
            )
        })
        .await
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        let account = self.keychain_account(name);
        storage_task(move || {
            pending_deletes::delete(&account, || backend::delete_secret_blocking(&account))
        })
        .await
    }
}

pub fn keychain_secrets(account: AccountId) -> Secrets {
    Arc::new(KeychainSecrets::new(account))
}

/// # Errors
///
/// Returns an error when no account is signed in. The settings page runs outside the Cortex webview
/// and cannot resolve an account itself, so this is the only account scope available to it.
pub fn active_secret_store(active: &ActiveAccount) -> Result<Secrets, String> {
    active
        .get()
        .map(keychain_secrets)
        .ok_or_else(|| "no account is signed in".to_string())
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemorySecrets {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
impl InMemorySecrets {
    pub fn with(entries: &[(&str, &str)]) -> Secrets {
        let entries = entries
            .iter()
            .map(|(name, secret)| ((*name).to_string(), (*secret).to_string()))
            .collect();
        Arc::new(InMemorySecrets {
            entries: std::sync::Mutex::new(entries),
        })
    }

    pub fn empty() -> Secrets {
        Arc::new(InMemorySecrets::default())
    }
}

/// A store whose reads are refused, as a denied keychain prompt would be.
#[cfg(test)]
pub struct DeniedSecrets;

#[cfg(test)]
impl DeniedSecrets {
    pub fn shared() -> Secrets {
        Arc::new(DeniedSecrets)
    }
}

#[cfg(test)]
#[async_trait]
impl SecretStore for DeniedSecrets {
    async fn store(&self, _name: &str, _secret: &str) -> Result<(), String> {
        Err("secure storage is unavailable".into())
    }

    async fn load(&self, _name: &str) -> Result<Option<String>, String> {
        Err("secure storage is unavailable".into())
    }

    async fn delete(&self, _name: &str) -> Result<(), String> {
        Err("secure storage is unavailable".into())
    }
}

#[cfg(test)]
#[async_trait]
impl SecretStore for InMemorySecrets {
    async fn store(&self, name: &str, secret: &str) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .insert(name.to_string(), secret.to_string());
        Ok(())
    }

    async fn load(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .get(name)
            .cloned())
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|_| "secret store lock poisoned".to_string())?
            .remove(name);
        Ok(())
    }
}

/// Deletes an account's secrets without reading its connector configuration.
pub fn delete_account_secrets_blocking(account: &AccountId) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::keychain::macos_queue::run_blocking(|| {
            backend::delete_account_secrets_blocking(account)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        backend::delete_account_secrets_blocking(account)
    }
}

#[cfg(any(not(windows), test))]
pub(super) fn secret_belongs_to_account(key: &str, account: &AccountId) -> bool {
    key.strip_prefix("connector:")
        .and_then(|key| key.split_once(':'))
        .is_some_and(|(owner, _)| owner == account.as_str())
}

pub async fn secure_storage_available() -> bool {
    storage_task(|| Ok(backend::secure_storage_available_blocking()))
        .await
        .unwrap_or(false)
}

/// Probed once at startup; a wrong answer is cleared by restarting the app.
pub struct SecureStorageStatus {
    pub available: bool,
}

#[cfg(test)]
mod tests {
    use super::{Activity, ConnectorStatus, KeychainSecrets, Readiness};
    use crate::account::AccountId;

    #[test]
    fn every_key_is_scoped_to_its_account() {
        let store = |id: &str| KeychainSecrets::new(AccountId::parse(id).unwrap());

        assert_eq!(
            store("1234").keychain_account("mcp-slack:token"),
            "connector:1234:mcp-slack:token"
        );
        assert_ne!(
            store("1234").keychain_account("mcp-slack"),
            store("5678").keychain_account("mcp-slack")
        );
    }

    #[test]
    fn secret_selection_matches_only_the_exact_account_namespace() {
        let account = AccountId::parse("abc").unwrap();
        assert!(super::secret_belongs_to_account(
            "connector:abc:mcp-slack:token",
            &account
        ));
        for key in [
            "connector:abcd:token",
            "connector:ABC:token",
            "connector:other:token",
            "connector:abc",
            "abc:token",
            "__storage_probe__",
        ] {
            assert!(
                !super::secret_belongs_to_account(key, &account),
                "{key} was selected"
            );
        }
    }

    const HAS_REMOTE: bool = true;
    const NO_REMOTE: bool = false;
    const HAS_WORKED_BEFORE: bool = true;
    const NEVER_WORKED: bool = false;

    #[test]
    fn a_connector_with_no_remote_is_judged_on_readiness_alone() {
        assert_eq!(
            ConnectorStatus::derive(
                &Readiness::Ready,
                NO_REMOTE,
                &Activity::Untried,
                NEVER_WORKED
            ),
            ConnectorStatus::Ready,
            "browser and a granted filesystem have no attempt history to consult"
        );
    }

    #[test]
    fn a_blocked_connector_surfaces_its_reason() {
        assert_eq!(
            ConnectorStatus::derive(
                &Readiness::blocked("Add folders first"),
                NO_REMOTE,
                &Activity::Untried,
                NEVER_WORKED,
            ),
            ConnectorStatus::NotReady {
                reason: "Add folders first".into()
            }
        );
    }

    #[test]
    fn a_connector_that_has_never_worked_cannot_read_as_ready() {
        let never_attempted = ConnectorStatus::derive(
            &Readiness::Ready,
            HAS_REMOTE,
            &Activity::Untried,
            NEVER_WORKED,
        );
        let attempted_and_failed = ConnectorStatus::derive(
            &Readiness::Ready,
            HAS_REMOTE,
            &Activity::Failed {
                error: "no such file or directory: npx".into(),
            },
            NEVER_WORKED,
        );

        assert_eq!(
            never_attempted,
            ConnectorStatus::NotReady {
                reason: "Not connected yet".into()
            }
        );
        assert_eq!(
            attempted_and_failed,
            ConnectorStatus::NotReady {
                reason: "no such file or directory: npx".into()
            },
            "a mistyped command must not present as a transient outage"
        );
    }

    #[test]
    fn a_connector_that_worked_before_degrades_to_a_warning_not_to_unconfigured() {
        assert_eq!(
            ConnectorStatus::derive(
                &Readiness::Ready,
                HAS_REMOTE,
                &Activity::Failed {
                    error: "connection refused".into()
                },
                HAS_WORKED_BEFORE,
            ),
            ConnectorStatus::Warning {
                error: "connection refused".into()
            }
        );
    }

    #[test]
    fn losing_credentials_outranks_a_remembered_success() {
        assert_eq!(
            ConnectorStatus::derive(
                &Readiness::blocked("Not connected"),
                HAS_REMOTE,
                &Activity::Succeeded,
                HAS_WORKED_BEFORE,
            ),
            ConnectorStatus::NotReady {
                reason: "Not connected".into()
            }
        );
    }
}
