mod catalog;
mod config;
mod credentials;
mod environment;
mod migrations;
mod sign_in;
mod tool_cache;
mod vetting;

pub use config::{
    load_mcp_servers, save_mcp_servers, validate_server_id, CustomHeader, HttpAuth,
    McpServerConfig, McpTransport,
};
#[cfg(test)]
use credentials::secret_keys;
pub use credentials::{destination_fingerprint, secret_keys_for_id};
use migrations::migrate_legacy_endpoints;
pub use tool_cache::{mcp_tools_path, NoToolCache, ToolCache};

use super::super::auth::{
    keychain_secrets, AuthMechanism, CredentialState, Interactive, McpTransportInfo, Readiness,
    Secrets, Stored,
};
use super::super::child_process::background_command;
use super::super::command_path;
use super::super::connector_state::ToolPolicy;
use super::super::error::{AuthError, ToolError};
use super::super::hidden_text;
use super::super::icons::bundled_icon_data_uri;
use super::super::oauth::{
    bind_oauth_callback, browser_url, wait_for_oauth_callback, DeepLinkCallback, SLACK_CALLBACK_URI,
};
use super::super::registry::{
    Connector, ConnectorDescriptor, Credentialed, Session, ToolDefinition,
};
use crate::account::AccountId;
use crate::reporting::mcp_result_serialization_failure;
use async_trait::async_trait;
use credentials::{
    client_secret_key, credential_state, env_secret_key, header_secret_key, token_key,
    BoundSecrets, KeychainCredentialStore, McpCredentialInput, UserAgentOAuthHttpClient,
};
use environment::{child_env, inherited_env};
use log::{debug, info, warn};
use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};
use rmcp::model::{CallToolRequestParams, ClientConfig, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::transport::auth::{
    default_oauth_http_client, AuthClient, AuthorizationManager, OAuthClientConfig,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tool_cache::AccountToolCache;
use vetting::{tool_definition, vet};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);
const MCP_OAUTH_REDIRECT_PORTS: [u16; 3] = [9876, 9877, 9878];

pub struct McpConnector {
    id: String,
    config: McpServerConfig,
    secrets: Secrets,
    tool_cache: Box<dyn ToolCache>,
    tools: RwLock<Vec<ToolDefinition>>,
    client: Mutex<Option<RunningService<RoleClient, ClientConfig>>>,
    credentials: RwLock<CredentialState>,
    call_timeout: Duration,
    catalog: bool,
}

impl McpConnector {
    pub fn new(
        config: McpServerConfig,
        secrets: Secrets,
        tool_cache: Box<dyn ToolCache>,
        credentials: CredentialState,
    ) -> Self {
        let catalog = catalog::is_catalog_server(&config);
        Self {
            id: format!("mcp-{}", config.id),
            secrets: Arc::new(BoundSecrets::new(secrets, &config)),
            config,
            tools: RwLock::new(Self::cached_tools(tool_cache.as_ref(), catalog)),
            tool_cache,
            client: Mutex::new(None),
            credentials: RwLock::new(credentials),
            call_timeout: DEFAULT_CALL_TIMEOUT,
            catalog,
        }
    }

    fn cached_tools(cache: &dyn ToolCache, catalog: bool) -> Vec<ToolDefinition> {
        cache
            .load()
            .into_iter()
            .map(|tool| vet(tool, catalog))
            .collect()
    }

    /// Probes the keychain once, which is what keeps [`Connector::readiness`] synchronous.
    pub async fn from_config(
        config: McpServerConfig,
        secrets: Secrets,
        app_handle: &AppHandle,
        account: &AccountId,
    ) -> Self {
        let credentials =
            credential_state(&config, &BoundSecrets::new(secrets.clone(), &config)).await;
        let tool_cache = Box::new(AccountToolCache {
            app_handle: app_handle.clone(),
            account: account.clone(),
            server_id: config.id.clone(),
        });
        Self::new(config, secrets, tool_cache, credentials)
    }

    pub fn with_call_timeout(
        config: McpServerConfig,
        secrets: Secrets,
        call_timeout: Duration,
    ) -> Self {
        Self {
            call_timeout,
            ..Self::new(
                config,
                secrets,
                Box::new(NoToolCache),
                CredentialState::Absent,
            )
        }
    }

    pub fn connector_id(&self) -> String {
        self.id.clone()
    }

    fn client_secret_key(&self) -> String {
        client_secret_key(&self.id)
    }

    fn token_key(&self) -> String {
        token_key(&self.id)
    }

    fn header_secret_key(&self, header_name: &str) -> String {
        header_secret_key(&self.id, header_name)
    }

    fn env_secret_key(&self, var_name: &str) -> String {
        env_secret_key(&self.id, var_name)
    }

    fn credentials(&self) -> CredentialState {
        *self
            .credentials
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_credentials(&self, state: CredentialState) {
        *self
            .credentials
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
    }

    /// Writes whatever the form supplied, reporting whether an access credential was written.
    ///
    /// A confidential client's secret is configuration rather than an access credential, so it
    /// does not count.
    async fn store_supplied(&self, supplied: &McpCredentialInput) -> Result<Stored, AuthError> {
        if supplied.is_empty() {
            return Ok(Stored::Nothing);
        }

        let mut wrote = Stored::Nothing;
        if let Some(secret) = supplied.client_secret.as_deref().filter(|s| !s.is_empty()) {
            self.secrets
                .store(&self.client_secret_key(), secret)
                .await
                .map_err(AuthError::Failed)?;
        }

        let auth = match &self.config.transport {
            McpTransport::Http { auth, .. } => auth,
            McpTransport::Stdio { secret_env, .. } => {
                for name in secret_env {
                    let Some(value) = supplied
                        .env
                        .get(name)
                        .map(String::as_str)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    self.secrets
                        .store(&self.env_secret_key(name), value)
                        .await
                        .map_err(AuthError::Failed)?;
                    wrote = Stored::Something;
                }
                return Ok(wrote);
            }
        };

        match auth {
            HttpAuth::Token { .. } => {
                if let Some(token) = supplied.token.as_deref().filter(|t| !t.is_empty()) {
                    self.secrets
                        .store(&self.token_key(), token)
                        .await
                        .map_err(AuthError::Failed)?;
                    wrote = Stored::Something;
                }
            }
            HttpAuth::Headers { headers } => {
                for header in headers {
                    let Some(value) = supplied
                        .headers
                        .get(&header.name)
                        .map(String::as_str)
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    self.secrets
                        .store(&self.header_secret_key(&header.name), value)
                        .await
                        .map_err(AuthError::Failed)?;
                    wrote = Stored::Something;
                }
            }
            HttpAuth::Oauth { .. } | HttpAuth::None => {}
        }
        Ok(wrote)
    }

    /// The config's `env` with each declared secret's keychain value laid over it.
    ///
    /// A declared secret that is not in the keychain is an error rather than an omission: the
    /// server would start without the credential it needs and fail its first call.
    fn parse_request_headers(
        request_headers: &HashMap<String, String>,
    ) -> Result<HashMap<HeaderName, HeaderValue>, ToolError> {
        let mut header_map = HashMap::new();
        for (name, value) in request_headers {
            if value.is_empty() {
                continue;
            }
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
                ToolError::ExecutionFailed(format!("invalid request header name '{name}': {e}"))
            })?;
            let value = HeaderValue::from_str(value).map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "invalid request header value for '{name}': {e}"
                ))
            })?;
            header_map.insert(name, value);
        }
        Ok(header_map)
    }

    fn mcp_client() -> ClientConfig {
        // serve() uses initialize, which the 2026-07-28 protocol removed.
        let mut config = ClientConfig::default();
        config.protocol_version = ProtocolVersion::LATEST_WITH_INITIALIZE;
        config
    }

    fn http_transport_config(
        url: &str,
        request_headers: &HashMap<String, String>,
    ) -> Result<StreamableHttpClientTransportConfig, ToolError> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        let headers = Self::parse_request_headers(request_headers)?;
        if !headers.is_empty() {
            config = config.custom_headers(headers);
        }
        Ok(config)
    }

    async fn resolved_env(
        &self,
        env: &HashMap<String, String>,
        secret_env: &[String],
    ) -> Result<HashMap<String, String>, ToolError> {
        let mut resolved = env.clone();
        for name in secret_env {
            let value = self
                .secrets
                .load(&self.env_secret_key(name))
                .await
                .map_err(ToolError::ExecutionFailed)?
                .filter(|value| !value.is_empty())
                .ok_or(ToolError::NotAuthenticated)?;
            resolved.insert(name.clone(), value);
        }
        Ok(resolved)
    }

    async fn ensure_connected(&self) -> Result<(), ToolError> {
        let mut guard = self.client.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        let service = match &self.config.transport {
            McpTransport::Stdio {
                command,
                args,
                env,
                secret_env,
            } => self.connect_stdio(command, args, env, secret_env).await?,
            McpTransport::Http {
                url,
                request_headers,
                auth: HttpAuth::Oauth {
                    scopes, client_id, ..
                },
            } => {
                self.connect_oauth(url, request_headers, scopes, client_id.as_deref())
                    .await?
            }
            McpTransport::Http {
                url,
                request_headers,
                auth: HttpAuth::Token { scheme },
            } => {
                let headers = self.token_headers(request_headers, scheme).await?;
                self.serve(StreamableHttpClientTransport::from_config(
                    StreamableHttpClientTransportConfig::with_uri(url.clone())
                        .custom_headers(headers),
                ))
                .await?
            }
            McpTransport::Http {
                url,
                request_headers,
                auth: HttpAuth::Headers { headers },
            } => {
                let headers = self.secret_headers(request_headers, headers).await?;
                self.serve(StreamableHttpClientTransport::from_config(
                    StreamableHttpClientTransportConfig::with_uri(url.clone())
                        .custom_headers(headers),
                ))
                .await?
            }
            McpTransport::Http {
                url,
                request_headers,
                auth: HttpAuth::None,
            } => {
                self.serve(StreamableHttpClientTransport::from_config(
                    Self::http_transport_config(url, request_headers)?,
                ))
                .await?
            }
        };

        let tools: Vec<ToolDefinition> = service
            .list_all_tools()
            .await
            .map_err(|e| ToolError::Unreachable(format!("{}: listing tools failed: {e}", self.id)))?
            .into_iter()
            .map(|tool| tool_definition(tool, self.catalog))
            .collect();

        self.tool_cache.save(&tools);
        *self
            .tools
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = tools;
        *guard = Some(service);
        Ok(())
    }

    /// Opens the MCP session over `transport`, giving up after [`CONNECT_TIMEOUT`].
    async fn serve<T, E, A>(
        &self,
        transport: T,
    ) -> Result<RunningService<RoleClient, ClientConfig>, ToolError>
    where
        T: rmcp::transport::IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        tokio::time::timeout(CONNECT_TIMEOUT, Self::mcp_client().serve(transport))
            .await
            .map_err(|_| {
                ToolError::Unreachable(format!(
                    "{}: no response within {CONNECT_TIMEOUT:?}",
                    self.id
                ))
            })?
            .map_err(|e| ToolError::Unreachable(format!("{}: initialize failed: {e}", self.id)))
    }

    /// Starts a local server with a cleared environment (what it is given, the few variables it
    /// inherits, and the login shell's PATH), logging what it writes to stderr.
    async fn connect_stdio(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        secret_env: &[String],
    ) -> Result<RunningService<RoleClient, ClientConfig>, ToolError> {
        let env = self.resolved_env(env, secret_env).await?;

        let resolved = command_path::resolve(command)
            .await
            .map_err(|e| ToolError::Unreachable(format!("{}: {e}", self.id)))?;

        let command = background_command(Command::new(&resolved.program).configure(|cmd| {
            cmd.args(args);
            cmd.env_clear();
            cmd.envs(inherited_env(std::env::vars_os()));
            cmd.envs(child_env(&env, &resolved.search_path));
        }));

        let (transport, stderr) = TokioChildProcess::builder(command)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                ToolError::Unreachable(format!("{}: failed to start server: {e}", self.id))
            })?;

        if let Some(stderr) = stderr {
            let connector_id = self.connector_id();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!("[{connector_id}] {line}");
                }
            });
        }

        self.serve(transport).await
    }

    /// Connects with the OAuth tokens kept for this server, refreshed by rmcp as they expire.
    async fn connect_oauth(
        &self,
        url: &str,
        request_headers: &HashMap<String, String>,
        scopes: &[String],
        client_id: Option<&str>,
    ) -> Result<RunningService<RoleClient, ClientConfig>, ToolError> {
        let mut manager = self
            .new_oauth_manager(url)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        let has_creds = manager.initialize_from_store().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("OAuth credential load failed: {e}"))
        })?;
        if !has_creds {
            return Err(ToolError::NotAuthenticated);
        }

        // Confidential clients: re-supply the client_secret so rmcp's
        // implicit token refresh can authenticate. initialize_from_store
        // only restores a secret-less client from the stored client_id.
        if let Some(client_id) = client_id {
            self.configure_confidential_client(
                &mut manager,
                client_id,
                scopes,
                &self.oauth_redirect_uri(MCP_OAUTH_REDIRECT_PORTS[0]),
            )
            .await
            .map_err(ToolError::ExecutionFailed)?;
        }

        let auth_client = AuthClient::new(reqwest::Client::new(), manager);
        self.serve(StreamableHttpClientTransport::with_client(
            auth_client,
            Self::http_transport_config(url, request_headers)?,
        ))
        .await
    }

    /// The request headers with `Authorization: <scheme> <token>`, from the token kept for this
    /// server.
    async fn token_headers(
        &self,
        request_headers: &HashMap<String, String>,
        scheme: &str,
    ) -> Result<HashMap<HeaderName, HeaderValue>, ToolError> {
        let token = self
            .secrets
            .load(&self.token_key())
            .await
            .map_err(ToolError::ExecutionFailed)?
            .filter(|v| !v.is_empty())
            .ok_or(ToolError::NotAuthenticated)?;

        let header = HeaderValue::from_str(&format!("{scheme} {token}")).map_err(|e| {
            ToolError::ExecutionFailed(format!("invalid Authorization header: {e}"))
        })?;
        let mut headers = Self::parse_request_headers(request_headers)?;
        headers.insert(AUTHORIZATION, header);
        Ok(headers)
    }

    /// The request headers with each secret header the server needs, from the values kept for it.
    async fn secret_headers(
        &self,
        request_headers: &HashMap<String, String>,
        headers: &[CustomHeader],
    ) -> Result<HashMap<HeaderName, HeaderValue>, ToolError> {
        let mut header_map = Self::parse_request_headers(request_headers)?;
        for spec in headers {
            let secret = self
                .secrets
                .load(&self.header_secret_key(&spec.name))
                .await
                .map_err(ToolError::ExecutionFailed)?
                .filter(|v| !v.is_empty())
                .ok_or(ToolError::NotAuthenticated)?;

            let name = HeaderName::from_bytes(spec.name.as_bytes()).map_err(|e| {
                ToolError::ExecutionFailed(format!("invalid header name '{}': {e}", spec.name))
            })?;
            let value =
                HeaderValue::from_str(&format!("{}{}", spec.prefix, secret)).map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "invalid value for header '{}': {e}",
                        spec.name
                    ))
                })?;
            header_map.insert(name, value);
        }
        Ok(header_map)
    }

    /// Ends the session and falls back to the cached tool list.
    async fn close_session(&self) {
        let service = self.client.lock().await.take();
        if let Some(service) = service {
            let _ = service.cancel().await;
        }
        *self
            .tools
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Self::cached_tools(self.tool_cache.as_ref(), self.catalog);
    }

    /// Separates a server that answered with an error from one the session could not reach.
    fn describe_call_failure(&self, error: ServiceError) -> ToolError {
        match error {
            ServiceError::TransportSend(_)
            | ServiceError::TransportClosed
            | ServiceError::Cancelled { .. } => {
                ToolError::Unreachable(format!("{}: {error}", self.id))
            }
            other => ToolError::ExecutionFailed(format!("MCP tool call failed: {other}")),
        }
    }

    async fn call_tool(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        let guard = self.client.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| ToolError::Unreachable(format!("{}: no session", self.id)))?;

        let mut params = CallToolRequestParams::new(tool_name.to_string());
        if let Some(arguments) = input.as_object() {
            params = params.with_arguments(arguments.clone());
        }

        let result = tokio::time::timeout(self.call_timeout, service.call_tool(params))
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "MCP tool '{tool_name}' timed out after {:?}",
                    self.call_timeout
                ))
            })?
            .map_err(|e| self.describe_call_failure(e))?;

        serde_json::to_value(result)
            .inspect_err(|_| mcp_result_serialization_failure())
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))
    }
}

#[async_trait]
impl Connector for McpConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn vetted(&self) -> bool {
        self.catalog
    }

    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: self.id.clone(),
            display_name: self.config.display_name.clone(),
            description: self.config.description.clone(),
            version: "1.0.0".into(),
            icon: bundled_icon_data_uri(&self.id),
            auth: AuthMechanism::McpServer {
                transport: match &self.config.transport {
                    McpTransport::Stdio {
                        command,
                        args,
                        env,
                        secret_env,
                    } => McpTransportInfo::Stdio {
                        command: command.clone(),
                        args: args.clone(),
                        env: env.clone(),
                        secret_env: secret_env.clone(),
                    },
                    McpTransport::Http { url, .. } => McpTransportInfo::Http { url: url.clone() },
                },
            },
        }
    }

    fn tools(&self) -> Vec<ToolDefinition> {
        self.tools
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn readiness(&self) -> Readiness {
        match &self.config.transport {
            McpTransport::Stdio { secret_env, .. } if secret_env.is_empty() => Readiness::Ready,
            McpTransport::Http {
                auth: HttpAuth::None,
                ..
            } => Readiness::Ready,
            _ => match self.credentials().blocked_reason() {
                Some(reason) => Readiness::blocked(reason),
                None => Readiness::Ready,
            },
        }
    }

    async fn invoke(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        self.ensure_connected().await?;
        self.call_tool(tool_name, input).await
    }

    async fn invoke_read_only(&self, tool_name: &str, input: Value) -> Result<Value, ToolError> {
        // Reconnection replaces cached metadata. Check the live catalog before calling.
        self.ensure_connected().await?;
        self.check_read_only(tool_name)?;
        self.call_tool(tool_name, input).await
    }

    /// A stdio server holds credentials only where its config declares secret environment
    /// variables; the values are supplied through the form and can be withdrawn.
    fn as_credentialed(&self) -> Option<&dyn Credentialed> {
        match &self.config.transport {
            McpTransport::Stdio { secret_env, .. } if secret_env.is_empty() => None,
            McpTransport::Http {
                auth: HttpAuth::None,
                ..
            } => None,
            _ => Some(self),
        }
    }

    fn as_session(&self) -> Option<&dyn Session> {
        Some(self)
    }

    async fn resources(&self) -> Result<Vec<Value>, ToolError> {
        self.ensure_connected().await?;
        let guard = self.client.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| ToolError::Unreachable(format!("{}: no session", self.id)))?;
        // A server without resources answers "method not found": it simply has none.
        if service
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_none())
        {
            return Ok(Vec::new());
        }
        let list = tokio::time::timeout(self.call_timeout, service.list_all_resources())
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!("{}: listing documents timed out", self.id))
            })?
            .map_err(|e| self.describe_call_failure(e))?;
        list.into_iter()
            .map(|r| serde_json::to_value(r).map_err(|e| ToolError::ExecutionFailed(e.to_string())))
            .collect()
    }

    async fn read_resource(&self, uri: &str) -> Result<String, ToolError> {
        self.ensure_connected().await?;
        let guard = self.client.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| ToolError::Unreachable(format!("{}: no session", self.id)))?;
        let result = tokio::time::timeout(
            self.call_timeout,
            service.read_resource(rmcp::model::ReadResourceRequestParams::new(uri)),
        )
        .await
        .map_err(|_| ToolError::ExecutionFailed(format!("{}: reading {uri} timed out", self.id)))?
        .map_err(|e| self.describe_call_failure(e))?;
        let value =
            serde_json::to_value(result).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(resource_text(&value))
    }
}

/// The text parts of a `resources/read` answer, joined; binary parts are skipped.
pub fn resource_text(result: &Value) -> String {
    result
        .get("contents")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

#[async_trait]
impl Session for McpConnector {
    async fn ensure(&self) -> Result<(), ToolError> {
        self.ensure_connected().await
    }

    async fn drop_session(&self) {
        self.close_session().await;
    }
}

pub async fn build_mcp_connectors(
    app_handle: &AppHandle,
    account: &AccountId,
) -> Result<Vec<McpConnector>, String> {
    let secrets = keychain_secrets(account.clone());
    let mut configs = load_mcp_servers(app_handle, account)?;
    let migrated = migrate_legacy_endpoints(&mut configs, secrets.as_ref()).await;
    if !migrated.is_empty() {
        match save_mcp_servers(app_handle, account, &configs) {
            Ok(()) => {
                for id in &migrated {
                    info!("{id}: moved to the current MCP endpoint; reconnect required");
                }
            }
            Err(e) => warn!("failed to save migrated MCP servers, will retry: {e}"),
        }
    }
    let mut connectors = Vec::with_capacity(configs.len());
    for config in crate::feature_flags::filter_mcp_presets(app_handle, configs) {
        connectors
            .push(McpConnector::from_config(config, secrets.clone(), app_handle, account).await);
    }
    Ok(connectors)
}

#[cfg(test)]
mod tests;
