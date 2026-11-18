use super::auth::{active_secret_store, Interactive, SecretStore, SecureStorageStatus};
use super::connector_state::{save_connector_state, ToolPolicy};
use super::connectors::mcp::{destination_fingerprint, validate_server_id};
use super::connectors::{load_mcp_servers, save_mcp_servers, McpConnector, McpServerConfig};
use super::registry::{
    ApprovalDecision, CallContext, ConnectorManifest, ToolDefinition, ToolRegistry,
};
use super::{emit_connector_changed, emit_connectors_changed};
use crate::account::{record_request_account, AccountId, ActiveAccount};
use crate::schedules::{bridge::ScheduleToolContext, coordinator::ScheduleCoordinator};
use log::{debug, info, warn};
use serde_json::Value;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

const TOOL_RESULT_LOG_LIMIT: usize = 4_000;

/// Whether two saved configurations point at the same server in the same way.
///
/// Display-name and description edits keep history; changing where or how it connects does not.
fn same_transport(before: &McpServerConfig, after: &McpServerConfig) -> bool {
    serde_json::to_value(&before.transport).ok() == serde_json::to_value(&after.transport).ok()
}

/// Whether stored secrets can stay: same destination and the same auth mode. A mode change
/// must clear them too, or a token left by `Token@A → None@A` would be sent on `→ Token@B`.
/// The keychain enforces the same rule on load, for edits that bypass this form.
fn keeps_credentials(before: &McpServerConfig, after: &McpServerConfig) -> bool {
    destination_fingerprint(before) == destination_fingerprint(after)
}

/// Keys to clear before saving `next`: none while credentials stay valid, else everything the
/// id may hold. A first save also clears, sweeping entries a removed server left behind.
fn stale_secret_keys(existing: Option<&McpServerConfig>, next: &McpServerConfig) -> Vec<String> {
    match existing {
        Some(previous) if keeps_credentials(previous, next) => vec![],
        Some(previous) => super::connectors::mcp::secret_keys_for_id(&next.id, &[previous, next]),
        None => super::connectors::mcp::secret_keys_for_id(&next.id, &[next]),
    }
}

/// Fails on the first error so the caller can leave the config untouched: the keychain
/// backends already treat a missing key as success, and defer a delete the keychain can't do
/// yet, so an error means the secret could still be loaded.
async fn delete_secrets(secrets: &dyn SecretStore, keys: &[String]) -> Result<(), String> {
    for key in keys {
        secrets
            .delete(key)
            .await
            .map_err(|e| format!("could not clear stored credentials: {e}"))?;
    }
    Ok(())
}

fn require_account(active: &ActiveAccount) -> Result<AccountId, String> {
    active
        .get()
        .ok_or_else(|| "no account is signed in".to_string())
}

fn truncate_for_log(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    format!(
        "{}… [truncated, {} bytes total]",
        &text[..text.floor_char_boundary(max_len)],
        text.len()
    )
}

fn log_tool_result(connector_id: &str, tool_name: &str, result: &Value) {
    if matches!(
        result.get("type").and_then(Value::as_str),
        Some("lumo_file" | "lumo_tool_result" | "lumo_file_image")
    ) || result
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("image")
                    || block
                        .get("resource")
                        .is_some_and(|resource| resource.get("blob").is_some())
            })
        })
    {
        debug!("Multimodal tool result contents omitted from logs");
        return;
    }
    match (connector_id, tool_name) {
        ("proton_mail" | "tool_results", _) | ("filesystem", "fs_read") => {
            debug!("Private tool result omitted from logs");
        }
        ("luminance", _) => {
            debug!("Luminance result omitted from logs");
        }
        ("browser", "navigate" | "fetch") => {
            if let Some(url) = result.get("url").and_then(Value::as_str) {
                debug!("page url: {url}");
            }
            if let Some(status) = result.get("status") {
                debug!("HTTP status: {status}");
            }
            if let Some(title) = result.get("title").and_then(Value::as_str) {
                debug!("page title: {title}");
            }
            if let Some(content) = result.get("content") {
                match content {
                    Value::String(text) => {
                        debug!(
                            "page content ({} bytes):\n{}",
                            text.len(),
                            truncate_for_log(text, TOOL_RESULT_LOG_LIMIT)
                        );
                    }
                    other => {
                        let rendered = other.to_string();
                        debug!(
                            "page content (json, {} bytes):\n{}",
                            rendered.len(),
                            truncate_for_log(&rendered, TOOL_RESULT_LOG_LIMIT)
                        );
                    }
                }
            }
            if let Some(links) = result.get("links").and_then(Value::as_array) {
                debug!("outbound links: {}", links.len());
            }
        }
        ("bash", "run") => {
            if let Some(workdir) = result.get("workdir").and_then(Value::as_str) {
                debug!("shell workdir: {workdir}");
            }
            if let Some(exit_code) = result.get("exit_code") {
                debug!("shell exit code: {exit_code}");
            }
            if let Some(stdout) = result.get("stdout").and_then(Value::as_str) {
                debug!(
                    "shell stdout ({} bytes):\n{}",
                    stdout.len(),
                    truncate_for_log(stdout, TOOL_RESULT_LOG_LIMIT)
                );
            }
            if let Some(stderr) = result.get("stderr").and_then(Value::as_str) {
                if !stderr.is_empty() {
                    debug!(
                        "shell stderr ({} bytes):\n{}",
                        stderr.len(),
                        truncate_for_log(stderr, TOOL_RESULT_LOG_LIMIT)
                    );
                }
            }
        }
        ("filesystem", "fs_write" | "fs_edit") => {
            let rendered = result.to_string();
            debug!(
                "file edit result ({} bytes): {}",
                rendered.len(),
                truncate_for_log(&rendered, TOOL_RESULT_LOG_LIMIT)
            );
        }
        _ => {
            let rendered = result.to_string();
            debug!(
                "invoke_tool result ({} bytes): {}",
                rendered.len(),
                truncate_for_log(&rendered, TOOL_RESULT_LOG_LIMIT)
            );
        }
    }
}

#[tauri::command]
pub fn is_secure_storage_available(status: State<'_, SecureStorageStatus>) -> bool {
    status.available
}

#[tauri::command]
pub async fn get_tool_registry(
    user_id: Option<String>,
    app_handle: AppHandle,
    registry: State<'_, ToolRegistry>,
) -> Result<Vec<ConnectorManifest>, String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    Ok(registry.all_manifests())
}

/// Harness tools are separate from user-managed connector manifests.
#[tauri::command]
pub async fn get_builtin_tools(
    user_id: Option<String>,
    app_handle: AppHandle,
    registry: State<'_, ToolRegistry>,
) -> Result<Vec<ToolDefinition>, String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    Ok(registry.builtin_tools())
}

#[tauri::command]
pub async fn set_connector_enabled(
    connector_id: String,
    enabled: bool,
    user_id: Option<String>,
    active: State<'_, ActiveAccount>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry.set_connector_enabled(&connector_id, enabled);
    save_connector_state(
        &app_handle,
        &require_account(&active)?,
        &registry.state_snapshot(),
    )?;
    emit_connectors_changed(&app_handle);
    Ok(())
}

#[tauri::command]
pub fn respond_tool_approval(request_id: String, approved: bool, always: Option<bool>) {
    let decision = match (approved, always.unwrap_or(false)) {
        (false, _) => ApprovalDecision::Deny,
        (true, false) => ApprovalDecision::Once,
        (true, true) => ApprovalDecision::Always,
    };
    super::approvals::respond(&request_id, decision);
}

#[tauri::command]
pub async fn set_tool_policy(
    connector_id: String,
    tool_name: String,
    policy: ToolPolicy,
    user_id: Option<String>,
    active: State<'_, ActiveAccount>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry.set_tool_policy(&connector_id, &tool_name, policy);
    save_connector_state(
        &app_handle,
        &require_account(&active)?,
        &registry.state_snapshot(),
    )?;
    emit_connectors_changed(&app_handle);
    Ok(())
}

/// Settings' "Always allow": the tool runs without asking, even after the conversation, or a
/// scheduled run, has read untrusted content.
#[tauri::command]
pub async fn always_allow_tool(
    connector_id: String,
    tool_name: String,
    user_id: Option<String>,
    webview: tauri::Webview,
    active: State<'_, ActiveAccount>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    if webview.label() != crate::settings::window::LABEL {
        return Err("tools can only be always allowed from Settings".into());
    }
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry.always_allow(&connector_id, &tool_name);
    save_connector_state(
        &app_handle,
        &require_account(&active)?,
        &registry.state_snapshot(),
    )?;
    emit_connectors_changed(&app_handle);
    Ok(())
}

/// Register permissions supplied by the signed-in schedule executor for one unsettled run.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Parameter names are the IPC contract with the web app.
pub async fn register_schedule_execution_context(
    run_id: String,
    connector_ids: Vec<String>,
    auto_approve: bool,
    read_only_connector_ids: Option<Vec<String>>,
    user_id: String,
    webview: tauri::Webview,
    app_handle: AppHandle,
    coordinator: State<'_, ScheduleCoordinator>,
) -> Result<ScheduleToolContext, String> {
    if webview.label() != crate::main_window::LABEL {
        return Err("schedule execution requires the main webview".into());
    }
    record_request_account(&app_handle, Some(&user_id)).await?;
    coordinator
        .register_execution_context(
            run_id,
            connector_ids,
            auto_approve,
            read_only_connector_ids.unwrap_or_default(),
        )
        .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Parameter names are the IPC contract with the web app.
pub async fn invoke_tool(
    connector_id: String,
    tool_name: String,
    input: Value,
    schedule_context: Option<ScheduleToolContext>,
    file_transport: Option<bool>,
    conversation_id: Option<String>,
    webview: tauri::Webview,
    coordinator: State<'_, ScheduleCoordinator>,
    user_id: Option<String>,
    app_handle: AppHandle,
    registry: State<'_, ToolRegistry>,
) -> Result<Value, String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    let run_id = schedule_context
        .as_ref()
        .map(|context| context.run_id.clone());
    let authorization = match schedule_context {
        Some(context) => {
            if webview.label() != crate::main_window::LABEL {
                return Err("schedule execution requires the main webview".into());
            }
            let authorization_connector = if connector_id == "tool_results"
                && matches!(tool_name.as_str(), "read" | "complete_file")
            {
                registry
                    .result_store
                    .source_connector(registry.result_store.scope(), &input)
                    .map_err(|e| e.to_ipc_string())?
            } else {
                connector_id.clone()
            };
            coordinator
                .authorize_tool(context, authorization_connector)
                .await?
        }
        None => crate::schedules::bridge::ToolAuthorization {
            auto_approve: false,
            read_only: false,
            connector_ids: None,
        },
    };
    info!("invoke_tool connector={connector_id} tool={tool_name}");
    if matches!(
        connector_id.as_str(),
        "proton_mail" | "filesystem" | "tool_results" | "luminance"
    ) {
        debug!("invoke_tool input=[omitted]");
    } else {
        debug!("invoke_tool input={input}");
    }
    let result = registry
        .dispatch_with_transport(
            &connector_id,
            &tool_name,
            input,
            authorization.auto_approve,
            authorization.read_only,
            file_transport.unwrap_or(false),
            CallContext {
                run: run_id.as_deref(),
                conversation: conversation_id.as_deref(),
                offered: authorization.connector_ids.as_deref(),
            },
        )
        .await
        .map_err(|e| {
            warn!(
                "invoke_tool {connector_id}.{tool_name} failed: {}",
                e.kind()
            );
            e.to_ipc_string()
        });
    if let Ok(ref value) = result {
        debug!("invoke_tool succeeded");
        if file_transport.unwrap_or(false) {
            if let Some(inner) = value.get("value") {
                log_tool_result(&connector_id, &tool_name, inner);
            }
        } else {
            log_tool_result(&connector_id, &tool_name, value);
        }
    }
    result
}

#[tauri::command]
pub async fn connect_connector(
    connector_id: String,
    credentials: Option<Value>,
    attempt_id: Option<String>,
    user_id: Option<String>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry
        .connect_attempt(
            &connector_id,
            credentials,
            Interactive::Yes,
            attempt_id.as_deref(),
        )
        .await
        .map_err(|e| {
            warn!("connect_connector {connector_id} failed: {}", e.kind());
            e.to_ipc_string()
        })?;

    emit_connector_changed(&app_handle, &connector_id);
    Ok(())
}

#[tauri::command]
pub async fn cancel_connector_connection(
    connector_id: String,
    attempt_id: String,
    user_id: Option<String>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry.cancel_connection(&connector_id, &attempt_id);
    Ok(())
}

#[tauri::command]
pub async fn disconnect_connector(
    connector_id: String,
    user_id: Option<String>,
    registry: State<'_, ToolRegistry>,
    app_handle: AppHandle,
) -> Result<(), String> {
    record_request_account(&app_handle, user_id.as_deref()).await?;
    registry
        .disconnect(&connector_id)
        .await
        .map_err(|e| e.to_ipc_string())?;
    emit_connector_changed(&app_handle, &connector_id);
    Ok(())
}

#[tauri::command]
pub async fn save_mcp_server(
    server: McpServerConfig,
    app_handle: AppHandle,
    active: State<'_, ActiveAccount>,
    registry: State<'_, ToolRegistry>,
) -> Result<(), String> {
    validate_server_id(&server.id)?;
    let connector_id = format!("mcp-{}", server.id);
    let _operation = registry
        .try_connector_operation(&connector_id)
        .map_err(|e| e.to_ipc_string())?;
    let account = require_account(&active)?;
    let mut servers = super::connectors::load_mcp_servers(&app_handle, &account)?;
    let existing = servers.iter().position(|s| s.id == server.id);
    let previous = existing.map(|index| &servers[index]);
    let transport_changed = previous.is_none_or(|previous| !same_transport(previous, &server));

    // Before the save: if a delete fails, the old config keeps the old secrets tied to the old
    // destination.
    let secrets = active_secret_store(&active)?;
    let stale_keys = stale_secret_keys(previous, &server);
    delete_secrets(secrets.as_ref(), &stale_keys).await?;

    match existing {
        Some(index) => servers[index] = server.clone(),
        None => servers.push(server.clone()),
    }
    save_mcp_servers(&app_handle, &account, &servers)?;

    if transport_changed {
        registry.forget_success_history(&connector_id);
    }
    use super::connectors::mcp::{HttpAuth, McpTransport};
    // A server that needs no sign-in is started once now, so its tools are listed and offered
    // in chats right away rather than after a manual connect.
    let starts_alone = matches!(
        server.transport,
        McpTransport::Stdio { .. }
            | McpTransport::Http {
                auth: HttpAuth::None,
                ..
            }
    );
    registry
        .replace(
            McpConnector::from_config(server, active_secret_store(&active)?, &app_handle, &account)
                .await,
        )
        .await;
    drop(_operation);
    if starts_alone {
        match registry
            .connect_attempt(&connector_id, None, Interactive::No, None)
            .await
        {
            Ok(()) => info!("{connector_id} started and listed its tools"),
            Err(e) => warn!("{connector_id} could not be started yet: {}", e.kind()),
        }
        emit_connector_changed(&app_handle, &connector_id);
    }
    Ok(())
}

#[tauri::command]
pub fn list_mcp_servers(
    app_handle: AppHandle,
    active: State<'_, ActiveAccount>,
) -> Result<Vec<McpServerConfig>, String> {
    load_mcp_servers(&app_handle, &require_account(&active)?)
}

#[tauri::command]
pub async fn remove_mcp_server(
    server_id: String,
    app_handle: AppHandle,
    active: State<'_, ActiveAccount>,
    registry: State<'_, ToolRegistry>,
) -> Result<(), String> {
    let connector_id = format!("mcp-{server_id}");
    let _operation = registry
        .try_connector_operation(&connector_id)
        .map_err(|e| e.to_ipc_string())?;
    let account = require_account(&active)?;
    let mut servers = load_mcp_servers(&app_handle, &account)?;
    let removed: Vec<&McpServerConfig> = servers.iter().filter(|s| s.id == server_id).collect();

    // Before the save, so a failed delete leaves the server listed rather than leaving its
    // secrets behind for the next server saved under this id.
    let secrets = active_secret_store(&active)?;
    let keys = super::connectors::mcp::secret_keys_for_id(&server_id, &removed);
    delete_secrets(secrets.as_ref(), &keys).await?;

    servers.retain(|s| s.id != server_id);
    save_mcp_servers(&app_handle, &account, &servers)?;

    let _ = registry.disconnect_locked(&connector_id).await;
    registry.retire(&connector_id).await;
    if let Ok(path) = super::connectors::mcp::mcp_tools_path(&app_handle, &account, &server_id) {
        let _ = std::fs::remove_file(path);
    }

    // Prune persisted disabled connector flag and per-tool policies so when a server is
    // later re-added it doesn't silently come back disabled or with stale policies.
    registry.set_connector_enabled(&connector_id, true);
    registry.clear_tool_policies(&connector_id);
    let _ = save_connector_state(&app_handle, &account, &registry.state_snapshot());

    emit_connector_changed(&app_handle, &connector_id);
    Ok(())
}

#[tauri::command]
pub async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select folder")
            .blocking_pick_folder()
            .map(|path| path.to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Rebuilds the connectors whose capabilities come from settings, so a saved grant takes effect
/// without a restart. Called only with an account signed in.
pub async fn on_settings_saved(
    app_handle: &AppHandle,
    registry: &ToolRegistry,
    account: &AccountId,
) {
    registry
        .replace(super::connectors::FilesystemConnector::from_settings(
            app_handle, account,
        ))
        .await;
    registry
        .replace(super::connectors::BashConnector::from_settings(
            app_handle, account,
        ))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::auth::{DeniedSecrets, InMemorySecrets};
    use crate::tools::connectors::mcp::{HttpAuth, McpTransport};

    #[test]
    fn log_preview_preserves_unicode_at_the_byte_limit() {
        let text = "a".repeat(3_999) + "é🦀";
        let preview = truncate_for_log(&text, 4_000);
        assert!(preview.starts_with(&"a".repeat(3_999)));
        assert!(preview.contains("4005 bytes total"));
        assert_eq!(truncate_for_log("🦀", 3), "… [truncated, 4 bytes total]");
        assert_eq!(truncate_for_log("é", 2), "é");
    }

    fn stdio(id: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            display_name: id.into(),
            description: None,
            transport: McpTransport::Stdio {
                command: command.into(),
                args: vec![],
                env: Default::default(),
                secret_env: vec![],
            },
        }
    }

    #[test]
    fn renaming_a_server_keeps_its_history_but_repointing_it_does_not() {
        let before = stdio("notion", "npx");
        let renamed = McpServerConfig {
            display_name: "Notion Workspace".into(),
            ..stdio("notion", "npx")
        };
        let repointed = stdio("notion", "npix");

        assert!(same_transport(&before, &renamed));
        assert!(!same_transport(&before, &repointed));
    }

    fn http(id: &str, url: &str, scopes: &[&str]) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            display_name: id.into(),
            description: None,
            transport: McpTransport::Http {
                url: url.into(),
                auth: crate::tools::connectors::mcp::HttpAuth::Oauth {
                    scopes: scopes.iter().map(|s| s.to_string()).collect(),
                    client_id: None,
                    resource: None,
                    authorize_params: Default::default(),
                },
                request_headers: Default::default(),
            },
        }
    }

    #[test]
    fn secrets_stay_with_a_server_only_while_it_points_at_the_same_place() {
        let before = http("github", "https://api.example.com/mcp", &["repo"]);
        let rescoped = http("github", "https://api.example.com/mcp", &["repo", "user"]);
        let repointed = http("github", "https://evil.example/mcp", &["repo"]);

        assert!(keeps_credentials(&before, &rescoped));
        assert!(!keeps_credentials(&before, &repointed));
        assert!(!keeps_credentials(&stdio("x", "npx"), &stdio("x", "sh")));
        assert!(!keeps_credentials(&before, &stdio("github", "npx")));
    }

    fn http_with(id: &str, url: &str, auth: HttpAuth) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            display_name: id.into(),
            description: None,
            transport: McpTransport::Http {
                url: url.into(),
                auth,
                request_headers: Default::default(),
            },
        }
    }

    fn token(url: &str) -> McpServerConfig {
        http_with(
            "github",
            url,
            HttpAuth::Token {
                scheme: "Bearer".into(),
            },
        )
    }

    fn no_auth(url: &str) -> McpServerConfig {
        http_with("github", url, HttpAuth::None)
    }

    async fn save(secrets: &dyn SecretStore, previous: &McpServerConfig, next: &McpServerConfig) {
        delete_secrets(secrets, &stale_secret_keys(Some(previous), next))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_token_does_not_survive_an_auth_mode_round_trip_to_another_url() {
        let secrets = InMemorySecrets::with(&[("mcp-github:token", "token-for-a")]);
        let (token_a, none_a, token_b) = (
            token("https://a.example/mcp"),
            no_auth("https://a.example/mcp"),
            token("https://b.example/mcp"),
        );

        save(secrets.as_ref(), &token_a, &none_a).await;
        save(secrets.as_ref(), &none_a, &token_b).await;

        assert_eq!(secrets.load("mcp-github:token").await.unwrap(), None);
    }

    #[tokio::test]
    async fn credentials_stay_while_the_destination_and_auth_mode_are_unchanged() {
        let secrets = InMemorySecrets::with(&[("mcp-github:token", "kept")]);
        let renamed = McpServerConfig {
            display_name: "GitHub".into(),
            ..token("https://a.example/mcp")
        };

        save(secrets.as_ref(), &token("https://a.example/mcp"), &renamed).await;

        assert_eq!(
            secrets.load("mcp-github:token").await.unwrap().as_deref(),
            Some("kept")
        );
    }

    #[tokio::test]
    async fn a_first_save_sweeps_secrets_left_under_the_same_id() {
        let secrets = InMemorySecrets::with(&[("mcp-github:token", "left-behind")]);
        let keys = stale_secret_keys(None, &token("https://b.example/mcp"));
        delete_secrets(secrets.as_ref(), &keys).await.unwrap();
        assert_eq!(secrets.load("mcp-github:token").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_delete_that_cannot_be_performed_or_deferred_stops_the_save() {
        let keys = stale_secret_keys(
            Some(&token("https://a.example/mcp")),
            &token("https://b.example/mcp"),
        );
        let denied = DeniedSecrets::shared();
        assert!(delete_secrets(denied.as_ref(), &keys).await.is_err());
    }

    #[test]
    fn server_ids_that_escape_the_tool_cache_or_keychain_namespace_are_rejected() {
        for id in [
            "chrome-devtools",
            "alpha-vantage",
            "my_server.v2",
            "Ünïcode",
        ] {
            assert!(validate_server_id(id).is_ok(), "{id}");
        }
        for id in ["", "../../x", "a/b", "a\\b", "a:token", "a\nb"] {
            assert!(validate_server_id(id).is_err(), "{id:?}");
        }
    }
}
