use super::keychain::{backend, pending_deletes};
use crate::account::{AccountId, ActiveAccount};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use tokio::task::spawn_blocking;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthMechanism {
    None,
    ApiKey {
        env_var: String,
    },
    BearerToken {
        source: TokenSource,
    },
    OAuth2 {
        client_id_env: String,
        client_secret_env: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
        redirect_port: u16,
    },
    McpServer {
        transport: McpTransportInfo,
    },
    ProtonBridge {
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<ProtonBridgeInfo>,
    },
    Luminance {
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<LuminanceInfo>,
    },
}

/// Where a Luminance connector points and how it signs in. Never carries the secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuminanceInfo {
    pub instance_url: String,
    pub auth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtonBridgeInfo {
    pub imap_port: u16,
    pub smtp_port: u16,
    pub username: String,
    pub imap_security: String,
    pub smtp_security: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "transport")]
pub enum McpTransportInfo {
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
        /// Names only. The values stay in the keychain and are not sent to the settings page.
        #[serde(default)]
        secret_env: Vec<String>,
    },
    Http {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TokenSource {
    EnvVar(String),
    Keychain { service: String, account: String },
    ConfigFile { key: String },
}

/// Whether a call is worth attempting.
///
/// Recomputed from connector-owned state on every read; never assigned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Readiness {
    Ready,
    Blocked { reason: String },
}

impl Readiness {
    pub fn blocked(reason: impl Into<String>) -> Self {
        Readiness::Blocked {
            reason: reason.into(),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Readiness::Ready)
    }
}

/// How the last attempt to reach a connector's remote went.
///
/// Registry-owned and per run. Advisory: nothing gates on it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Activity {
    #[default]
    Untried,
    Succeeded,
    Failed {
        error: String,
    },
}

/// What the connector list renders, composed from readiness, activity and whether the connector
/// has ever succeeded.
///
/// For a connector with a remote, `Ready` requires a past success.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ConnectorStatus {
    Ready,
    Warning { error: String },
    NotReady { reason: String },
}

impl ConnectorStatus {
    pub fn derive(
        readiness: &Readiness,
        has_remote: bool,
        activity: &Activity,
        has_ever_succeeded: bool,
    ) -> Self {
        match readiness {
            Readiness::Blocked { reason } => {
                return ConnectorStatus::NotReady {
                    reason: reason.clone(),
                }
            }
            Readiness::Ready if !has_remote => return ConnectorStatus::Ready,
            Readiness::Ready => {}
        }
        match (has_ever_succeeded, activity) {
            (true, Activity::Failed { error }) => ConnectorStatus::Warning {
                error: error.clone(),
            },
            (true, _) => ConnectorStatus::Ready,
            (false, Activity::Failed { error }) => ConnectorStatus::NotReady {
                reason: error.clone(),
            },
            (false, _) => ConnectorStatus::NotReady {
                reason: "Not connected yet".into(),
            },
        }
    }
}

/// Whether a connector's stored credentials could be found.
///
/// `Unreadable` is separate from `Absent` because a keychain read the user has denied must not be
/// reported as a connector that was never set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialState {
    Present,
    Absent,
    Unreadable,
}

impl CredentialState {
    pub fn of(loaded: Result<Option<String>, String>) -> Self {
        match loaded {
            Ok(Some(secret)) if !secret.is_empty() => CredentialState::Present,
            Ok(_) => CredentialState::Absent,
            Err(_) => CredentialState::Unreadable,
        }
    }

    /// Combines the states of several secrets a connector needs together.
    ///
    /// Not knowing outranks knowing something is missing, so one unreadable secret makes the whole
    /// set unreadable.
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (CredentialState::Unreadable, _) | (_, CredentialState::Unreadable) => {
                CredentialState::Unreadable
            }
            (CredentialState::Absent, _) | (_, CredentialState::Absent) => CredentialState::Absent,
            _ => CredentialState::Present,
        }
    }

    pub fn blocked_reason(self) -> Option<&'static str> {
        match self {
            CredentialState::Present => None,
            CredentialState::Absent => Some("Not connected"),
            CredentialState::Unreadable => Some("Secure storage is unavailable"),
        }
    }
}

/// Whether an operation may prompt the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interactive {
    Yes,
    No,
}

/// Whether [`Credentialed::authenticate`] wrote a credential that has not yet been proven.
///
/// [`Credentialed::authenticate`]: crate::tools::registry::Credentialed::authenticate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Nothing,
    Something,
}

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

/// Reads and writes one account's connector secrets.
///
/// A connector holds the store it was built with rather than looking up whichever account is
/// current, so an OAuth callback that lands after the user has switched still writes to the
/// account it was started under.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn store(&self, name: &str, secret: &str) -> Result<(), String>;
    async fn load(&self, name: &str) -> Result<Option<String>, String>;
    async fn delete(&self, name: &str) -> Result<(), String>;
}

pub type Secrets = Arc<dyn SecretStore>;

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
