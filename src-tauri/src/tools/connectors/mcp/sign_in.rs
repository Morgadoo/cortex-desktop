//! Signing in to a server: OAuth (with a loopback or deep-link callback), and the
//! `Credentialed` side of the connector that stores and forgets what the user supplies.

use super::*;

impl McpConnector {
    pub(super) fn oauth_redirect_uri(&self, port: u16) -> String {
        if self.uses_slack_callback() {
            SLACK_CALLBACK_URI.to_string()
        } else {
            format!("http://127.0.0.1:{port}/callback")
        }
    }

    pub(super) fn uses_slack_callback(&self) -> bool {
        matches!(&self.config.transport, McpTransport::Http { url, .. }
            if url::Url::parse(url).is_ok_and(|url|
                url.scheme() == "https" && url.host_str() == Some("mcp.slack.com")
                && url.path().trim_end_matches('/') == "/mcp"))
    }

    /// Builds an `AuthorizationManager` with the keychain credential store and
    /// server metadata already discovered, so callers never depend on another
    /// method's discovery side effects.
    pub(super) async fn new_oauth_manager(
        &self,
        oauth_base: &str,
    ) -> Result<AuthorizationManager, String> {
        let http_client =
            default_oauth_http_client().map_err(|e| format!("OAuth init failed: {e}"))?;
        let mut manager = AuthorizationManager::new_with_oauth_http_client(
            oauth_base.to_string(),
            std::sync::Arc::new(UserAgentOAuthHttpClient(http_client)),
        )
        .await
        .map_err(|e| format!("OAuth init failed: {e}"))?;
        manager.set_credential_store(KeychainCredentialStore::new(
            self.secrets.clone(),
            self.connector_id(),
        ));

        let resolution = manager
            .resolve_metadata()
            .await
            .map_err(|e| format!("OAuth metadata discovery failed: {e}"))?;
        manager.set_metadata(resolution.metadata);

        Ok(manager)
    }

    /// Configures an explicit-`client_id` (confidential) OAuth client, attaching
    /// the stored `client_secret` when one is present.
    ///
    /// Must be called after server metadata is set and, on the reconnect path,
    /// after `initialize_from_store`, which would otherwise install a secret-less
    /// client that clobbers this configuration.
    pub(super) async fn configure_confidential_client(
        &self,
        manager: &mut AuthorizationManager,
        client_id: &str,
        scopes: &[String],
        redirect_uri: &str,
    ) -> Result<(), String> {
        let mut config = OAuthClientConfig::new(client_id.to_string(), redirect_uri.to_string())
            .with_scopes(scopes.to_vec());
        if let Some(secret) = self
            .secrets
            .load(&self.client_secret_key())
            .await?
            .filter(|v| !v.is_empty())
        {
            config = config.with_client_secret(secret);
        }
        manager
            .configure_client(config)
            .map_err(|e| format!("client configuration failed: {e}"))
    }

    pub(super) fn append_authorize_params(
        auth_url: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        if params.is_empty() {
            return Ok(auth_url.to_string());
        }
        let mut parsed =
            url::Url::parse(auth_url).map_err(|e| format!("invalid authorization URL: {e}"))?;
        {
            let mut pairs = parsed.query_pairs_mut();
            for (key, value) in params {
                pairs.append_pair(key, value);
            }
        }
        Ok(parsed.to_string())
    }

    pub(super) async fn authenticate_http(
        &self,
        oauth_base: &str,
        scopes: &[String],
        client_id: Option<&str>,
        authorize_params: &HashMap<String, String>,
    ) -> Result<(), AuthError> {
        let listener = if self.uses_slack_callback() {
            None
        } else {
            Some(bind_oauth_callback(&MCP_OAUTH_REDIRECT_PORTS).await?)
        };
        let redirect_uri = match &listener {
            Some(listener) => self.oauth_redirect_uri(
                listener
                    .local_addr()
                    .map_err(|e| AuthError::Failed(e.to_string()))?
                    .port(),
            ),
            None => SLACK_CALLBACK_URI.to_string(),
        };

        let mut manager = self
            .new_oauth_manager(oauth_base)
            .await
            .map_err(AuthError::Failed)?;

        let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();

        match client_id {
            Some(client_id) => {
                self.configure_confidential_client(&mut manager, client_id, scopes, &redirect_uri)
                    .await
                    .map_err(AuthError::Failed)?;
            }
            None => {
                manager
                    .register_client("Cortex Desktop", &redirect_uri, &scope_refs)
                    .await
                    .map_err(|e| AuthError::Failed(format!("client registration failed: {e}")))?;
            }
        }

        let auth_url = manager
            .get_authorization_url(&scope_refs)
            .await
            .map_err(|e| AuthError::Failed(format!("authorization URL failed: {e}")))?;
        let auth_url = Self::append_authorize_params(&auth_url, authorize_params)
            .map_err(AuthError::Failed)?;
        let auth_url = browser_url(&auth_url)?.to_string();
        let (code, state, issuer) = match listener {
            Some(listener) => {
                open::that(&auth_url).map_err(|e| {
                    AuthError::Failed(format!("failed to open browser: {:?}", e.kind()))
                })?;
                wait_for_oauth_callback(listener).await?
            }
            None => {
                let callback = DeepLinkCallback::register(&auth_url)?;
                open::that(&auth_url).map_err(|e| {
                    AuthError::Failed(format!("failed to open browser: {:?}", e.kind()))
                })?;
                callback.wait().await?
            }
        };
        manager
            .exchange_code_for_token_with_issuer(&code, &state, issuer.as_deref())
            .await
            .map_err(|e| AuthError::Failed(format!("token exchange failed: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl Credentialed for McpConnector {
    async fn authenticate(
        &self,
        input: Option<Value>,
        mode: Interactive,
    ) -> Result<Stored, AuthError> {
        let supplied = McpCredentialInput::parse(input)?;

        let wrote = self.store_supplied(&supplied).await?;
        let state = credential_state(&self.config, self.secrets.as_ref()).await;
        self.set_credentials(state);
        match state {
            CredentialState::Present => return Ok(wrote),
            CredentialState::Unreadable => {
                return Err(AuthError::Failed(
                    "secure storage is unavailable, so stored credentials could not be read".into(),
                ))
            }
            CredentialState::Absent => {}
        }

        // Only an HTTP transport has a flow to start: a secret environment variable is supplied
        // through the form or not at all.
        let McpTransport::Http { url, auth, .. } = &self.config.transport else {
            return Err(AuthError::RequiresSetup);
        };

        match auth {
            HttpAuth::Oauth {
                scopes,
                client_id,
                resource: _,
                authorize_params,
            } => {
                if mode == Interactive::No {
                    return Err(AuthError::RequiresSetup);
                }
                self.authenticate_http(url, scopes, client_id.as_deref(), authorize_params)
                    .await?;
                self.set_credentials(CredentialState::Present);
                Ok(Stored::Something)
            }
            HttpAuth::Token { .. } | HttpAuth::Headers { .. } => Err(AuthError::RequiresSetup),
            HttpAuth::None => Ok(wrote),
        }
    }

    async fn forget(&self) -> Result<(), String> {
        match &self.config.transport {
            McpTransport::Stdio { secret_env, .. } => {
                for name in secret_env {
                    self.secrets.delete(&self.env_secret_key(name)).await?;
                }
            }
            McpTransport::Http { auth, .. } => {
                self.secrets.delete(&self.id).await?;
                self.secrets.delete(&self.token_key()).await?;
                if let HttpAuth::Headers { headers } = auth {
                    for header in headers {
                        self.secrets
                            .delete(&self.header_secret_key(&header.name))
                            .await?;
                    }
                }
            }
        }
        self.set_credentials(CredentialState::Absent);
        Ok(())
    }
}
