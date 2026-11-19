//! Where an MCP server's secrets live, and how the settings form's input reaches them.

use super::config::{HttpAuth, McpServerConfig, McpTransport};
use crate::reporting::credential_failure;
use crate::tools::auth::{CredentialState, SecretStore, Secrets};
use crate::tools::error::AuthError;
use async_trait::async_trait;
use log::warn;
use reqwest::header::{HeaderValue, USER_AGENT};
use rmcp::transport::auth::{
    AuthError as OAuthError, CredentialStore, OAuthHttpClient, OAuthHttpClientFuture,
    OAuthHttpRequest, StoredCredentials,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

const OAUTH_USER_AGENT: &str = concat!("Cortex-Desktop/", env!("CARGO_PKG_VERSION"));

/// rmcp's default OAuth client, sending a `User-Agent`. Some authorization servers (Brandfetch
/// behind Vercel) answer 403 to requests without one, which discovery reports as no metadata.
pub(super) struct UserAgentOAuthHttpClient<C>(pub(super) C);

impl<C: OAuthHttpClient> OAuthHttpClient for UserAgentOAuthHttpClient<C> {
    fn execute(&self, mut request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        request
            .request
            .headers_mut()
            .entry(USER_AGENT)
            .or_insert(HeaderValue::from_static(OAUTH_USER_AGENT));
        self.0.execute(request)
    }
}

pub(super) struct KeychainCredentialStore {
    secrets: Secrets,
    name: String,
}

impl KeychainCredentialStore {
    pub(super) fn new(secrets: Secrets, name: impl Into<String>) -> Self {
        Self {
            secrets,
            name: name.into(),
        }
    }
}

#[async_trait]
impl CredentialStore for KeychainCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, OAuthError> {
        match self
            .secrets
            .load(&self.name)
            .await
            .map_err(OAuthError::InternalError)?
        {
            Some(json) => serde_json::from_str(&json)
                .map(Some)
                .inspect_err(|_| credential_failure("Credential decoding failed"))
                .map_err(|e| OAuthError::InternalError(e.to_string())),
            None => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), OAuthError> {
        let json = serde_json::to_string(&credentials)
            .inspect_err(|_| credential_failure("Credential serialization failed"))
            .map_err(|e| OAuthError::InternalError(e.to_string()))?;
        self.secrets
            .store(&self.name, &json)
            .await
            .map_err(OAuthError::InternalError)
    }

    async fn clear(&self) -> Result<(), OAuthError> {
        self.secrets
            .delete(&self.name)
            .await
            .map_err(OAuthError::InternalError)
    }
}

/// What a server's secrets are released to: the URL and auth mode, or the program, arguments
/// and environment of a stdio server. Mirrors what the settings form clears secrets on.
pub fn destination_fingerprint(config: &McpServerConfig) -> String {
    let canonical = match &config.transport {
        McpTransport::Http { url, auth, .. } => {
            let mode = match auth {
                HttpAuth::Oauth { .. } => "oauth",
                HttpAuth::Token { .. } => "token",
                HttpAuth::Headers { .. } => "headers",
                HttpAuth::None => "none",
            };
            serde_json::json!(["http", url, mode])
        }
        McpTransport::Stdio {
            command, args, env, ..
        } => {
            let env: BTreeMap<_, _> = env.iter().collect();
            serde_json::json!(["stdio", command, args, env])
        }
    };
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

pub(super) fn binding_key(connector_id: &str) -> String {
    format!("{connector_id}:binding")
}

/// A server's secrets, released only while its config names the destination they were saved
/// for.
///
/// The settings form clears secrets when the destination changes, but the config file can be
/// edited without it (by a shell tool following injected instructions, say). Such an edit must
/// not send the old server's credentials to the new destination.
pub(super) struct BoundSecrets {
    inner: Secrets,
    server_id: String,
    binding_key: String,
    fingerprint: String,
    config: McpServerConfig,
}

impl BoundSecrets {
    pub(super) fn new(inner: Secrets, config: &McpServerConfig) -> Self {
        Self {
            inner,
            server_id: config.id.clone(),
            binding_key: binding_key(&format!("mcp-{}", config.id)),
            fingerprint: destination_fingerprint(config),
            config: config.clone(),
        }
    }
}

#[async_trait]
impl SecretStore for BoundSecrets {
    async fn store(&self, name: &str, secret: &str) -> Result<(), String> {
        let bound = self.inner.load(&self.binding_key).await?;
        if bound
            .as_deref()
            .is_some_and(|bound| bound != self.fingerprint)
        {
            // Rebinding releases whatever is stored, so first drop what the old destination left.
            for key in secret_keys_for_id(&self.server_id, &[&self.config]) {
                if key != self.binding_key {
                    self.inner.delete(&key).await?;
                }
            }
        }
        if bound.as_deref() != Some(self.fingerprint.as_str()) {
            self.inner
                .store(&self.binding_key, &self.fingerprint)
                .await?;
        }
        self.inner.store(name, secret).await
    }

    async fn load(&self, name: &str) -> Result<Option<String>, String> {
        match self.inner.load(&self.binding_key).await? {
            Some(bound) if bound == self.fingerprint => self.inner.load(name).await,
            Some(_) => {
                warn!(
                    "mcp-{}: saved credentials were for a different destination; reconnect to use this one",
                    self.server_id
                );
                Ok(None)
            }
            // Saved before bindings existed: adopt the destination they are used with now.
            None => {
                let secret = self.inner.load(name).await?;
                if secret.is_some() {
                    self.inner
                        .store(&self.binding_key, &self.fingerprint)
                        .await?;
                }
                Ok(secret)
            }
        }
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        self.inner.delete(name).await
    }
}

/// Secrets typed into the settings form, carried to the keychain by `authenticate`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct McpCredentialInput {
    #[serde(default)]
    pub(super) token: Option<String>,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
    #[serde(default)]
    pub(super) headers: HashMap<String, String>,
    /// Values for the names declared in the stdio transport's `secret_env`, keyed by name.
    #[serde(default)]
    pub(super) env: HashMap<String, String>,
}

impl McpCredentialInput {
    pub(super) fn parse(input: Option<Value>) -> Result<Self, AuthError> {
        match input {
            None | Some(Value::Null) => Ok(Self::default()),
            Some(input) => serde_json::from_value(input)
                .map_err(|e| AuthError::Failed(format!("invalid MCP credentials: {e}"))),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.token.is_none()
            && self.client_secret.is_none()
            && self.headers.is_empty()
            && self.env.is_empty()
    }
}

/// Whether every secret this transport needs is in the store.
///
/// A stdio server needs one per name in `secret_env`.
pub(super) async fn credential_state(
    config: &McpServerConfig,
    secrets: &dyn SecretStore,
) -> CredentialState {
    let id = format!("mcp-{}", config.id);
    match &config.transport {
        McpTransport::Stdio { secret_env, .. } => {
            let mut state = CredentialState::Present;
            for name in secret_env {
                let loaded = secrets.load(&env_secret_key(&id, name)).await;
                state = state.and(CredentialState::of(loaded));
            }
            state
        }
        McpTransport::Http {
            auth: HttpAuth::Oauth { .. },
            ..
        } => CredentialState::of(secrets.load(&id).await),
        McpTransport::Http {
            auth: HttpAuth::Token { .. },
            ..
        } => CredentialState::of(secrets.load(&token_key(&id)).await),
        McpTransport::Http {
            auth: HttpAuth::Headers { headers },
            ..
        } => {
            if headers.is_empty() {
                return CredentialState::Absent;
            }
            let mut state = CredentialState::Present;
            for header in headers {
                let loaded = secrets.load(&header_secret_key(&id, &header.name)).await;
                state = state.and(CredentialState::of(loaded));
            }
            state
        }
        McpTransport::Http {
            auth: HttpAuth::None,
            ..
        } => CredentialState::Present,
    }
}

pub(super) fn token_key(connector_id: &str) -> String {
    format!("{connector_id}:token")
}

pub(super) fn header_secret_key(connector_id: &str, header_name: &str) -> String {
    format!("{connector_id}:header:{header_name}")
}

pub(super) fn env_secret_key(connector_id: &str, var_name: &str) -> String {
    format!("{connector_id}:env:{var_name}")
}

pub(super) fn client_secret_key(connector_id: &str) -> String {
    format!("{connector_id}:client_secret")
}

/// Every key that could hold a secret for `id` under any of `configs`, plus the fixed keys any
/// auth mode may have written.
///
/// Entries whose names appear in none of `configs` (a header or env var from an older config)
/// cannot be listed, as the keychain has no enumeration.
pub fn secret_keys_for_id(id: &str, configs: &[&McpServerConfig]) -> Vec<String> {
    let connector_id = format!("mcp-{id}");
    let mut keys = vec![
        connector_id.clone(),
        token_key(&connector_id),
        client_secret_key(&connector_id),
        binding_key(&connector_id),
    ];
    for config in configs {
        keys.extend(secret_keys(config));
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Every keychain entry a saved server owns, for deletion when the server is removed.
///
/// Wider than what `Credentialed::forget` withdraws: this includes the confidential client's
/// secret, which is configuration that survives a disconnect but not a removal. A key absent from
/// this list outlives the server and is inherited by the next server saved under the same id.
pub fn secret_keys(config: &McpServerConfig) -> Vec<String> {
    let id = format!("mcp-{}", config.id);
    match &config.transport {
        McpTransport::Stdio { secret_env, .. } => secret_env
            .iter()
            .map(|name| env_secret_key(&id, name))
            .collect(),
        McpTransport::Http { auth, .. } => match auth {
            HttpAuth::None => vec![],
            HttpAuth::Oauth { .. } => {
                vec![id.clone(), token_key(&id), client_secret_key(&id)]
            }
            HttpAuth::Token { .. } => vec![token_key(&id)],
            HttpAuth::Headers { headers } => {
                let mut keys = vec![id.clone(), token_key(&id), client_secret_key(&id)];
                keys.extend(
                    headers
                        .iter()
                        .map(|header| header_secret_key(&id, &header.name)),
                );
                keys
            }
        },
    }
}
