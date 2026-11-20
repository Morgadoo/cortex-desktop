//! What a connector says about its credentials and readiness, and the store its secrets live in.
//! The OS keychain behind that store is the app's (`tools::auth`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
/// [`Credentialed::authenticate`]: crate::registry::Credentialed::authenticate
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Nothing,
    Something,
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
