//! Tools: connectors, OpenAPI servers, custom tools, computer use, the sandbox, the vault, hooks, functions and privacy.

use super::*;

/// The agent workbench (workbench.rs): whether it is on, and its state.
#[tauri::command]
pub async fn chat_workbench_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let dir = config_dir(&app)?;
    Ok(serde_json::json!({
        "enabled": crate::chat::workbench::enabled(&dir),
        "image": crate::chat::workbench::image_tag(),
        "status": crate::chat::workbench::status().await,
    }))
}

#[tauri::command]
pub fn chat_workbench_set_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    crate::chat::workbench::set_enabled(&config_dir(&app)?, enabled).map_err(String::from)
}

/// Builds the workbench image (a large download, so only when the user asks).
#[tauri::command]
pub async fn chat_workbench_build(
    app: AppHandle,
) -> Result<crate::chat::workbench::Status, String> {
    Ok(crate::chat::workbench::build(&config_dir(&app)?).await)
}

#[tauri::command]
pub fn chat_functions(app: AppHandle) -> Result<Vec<crate::chat::functions::Function>, String> {
    Ok(crate::chat::functions::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_function(
    app: AppHandle,
    function: crate::chat::functions::Function,
) -> Result<Vec<crate::chat::functions::Function>, String> {
    crate::chat::functions::save(&config_dir(&app)?, function).map_err(String::from)
}

#[tauri::command]
pub fn chat_delete_function(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::functions::Function>, String> {
    crate::chat::functions::delete(&config_dir(&app)?, &id).map_err(String::from)
}

/// Runs a function on a sample, without saving it, to see what it does.
#[tauri::command]
pub async fn chat_test_function(
    app: AppHandle,
    function: crate::chat::functions::Function,
    sample: String,
) -> Result<String, String> {
    let dir = config_dir(&app)?;
    let args = if function.kind == "pipe" {
        serde_json::json!({ "messages": [{ "role": "user", "content": sample }], "model": function.id })
    } else {
        serde_json::json!({ "stage": "inlet", "text": sample, "model": "test" })
    };
    let out = crate::chat::functions::run(&dir, "function-test", &function, &args).await?;
    Ok(
        match (
            function.kind.as_str(),
            crate::chat::functions::verdict(&out),
        ) {
            ("filter", crate::chat::functions::Verdict::Keep) => "Kept as it was.".into(),
            ("filter", crate::chat::functions::Verdict::Block(reason)) => {
                format!("Refused: {reason}")
            }
            ("filter", crate::chat::functions::Verdict::Replace(t)) => format!("Becomes: {t}"),
            _ => out,
        },
    )
}

/// The vault's secrets (names and notes; values stay in the keychain).
#[tauri::command]
pub fn chat_vault_list(app: AppHandle) -> Result<Vec<crate::chat::vault::Secret>, String> {
    Ok(crate::chat::vault::list(&config_dir(&app)?))
}

/// Adds or updates a vault secret; `value` is stored in the keychain when given.
#[tauri::command]
pub async fn chat_vault_save(
    app: AppHandle,
    secret: crate::chat::vault::Secret,
    value: Option<String>,
) -> Result<Vec<crate::chat::vault::Secret>, String> {
    crate::chat::vault::save(&config_dir(&app)?, secret, value)
        .await
        .map_err(String::from)
}

/// Removes a vault secret, its stored value, and every agent's access to it.
#[tauri::command]
pub async fn chat_vault_delete(
    app: AppHandle,
    name: String,
) -> Result<Vec<crate::chat::vault::Secret>, String> {
    let dir = config_dir(&app)?;
    for mut agent in crate::chat::agents::load(&dir)
        .into_iter()
        .filter(|a| a.secrets.contains(&name))
    {
        agent.secrets.retain(|s| s != &name);
        crate::chat::agents::save(&dir, agent)?;
    }
    crate::chat::vault::delete(&dir, &name)
        .await
        .map_err(String::from)
}

/// The user's hooks.
#[tauri::command]
pub fn chat_get_hooks(app: AppHandle) -> Result<Vec<crate::chat::hooks::Hook>, String> {
    Ok(crate::chat::hooks::load(&config_dir(&app)?))
}

/// Saves the user's hooks.
#[tauri::command]
pub fn chat_save_hooks(app: AppHandle, hooks: Vec<crate::chat::hooks::Hook>) -> Result<(), String> {
    crate::chat::hooks::save(&config_dir(&app)?, &hooks).map_err(String::from)
}

#[tauri::command]
pub fn chat_openapi_servers(app: AppHandle) -> Result<Vec<crate::chat::openapi::Server>, String> {
    Ok(crate::chat::openapi::list(&config_dir(&app)?))
}

/// Adds an OpenAPI tool server from its spec address, with an optional bearer key.
#[tauri::command]
pub async fn chat_add_openapi_server(
    app: AppHandle,
    name: String,
    spec_url: String,
    key: Option<String>,
) -> Result<Vec<crate::chat::openapi::Server>, String> {
    let dir = config_dir(&app)?;
    let servers = crate::chat::openapi::add(&dir, &name, &spec_url).await?;
    match (
        key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
        servers.last(),
    ) {
        (Some(key), Some(added)) => {
            secrets()?
                .store(&crate::chat::openapi::secret_name(&added.id), &key)
                .await?;
            crate::chat::openapi::update(&dir, &added.id.clone(), None, Some(true))
                .map_err(String::from)
        }
        _ => Ok(servers),
    }
}

#[tauri::command]
pub async fn chat_refresh_openapi_server(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::openapi::Server>, String> {
    crate::chat::openapi::refresh(&config_dir(&app)?, &id)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub fn chat_toggle_openapi_server(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<Vec<crate::chat::openapi::Server>, String> {
    crate::chat::openapi::update(&config_dir(&app)?, &id, Some(enabled), None).map_err(String::from)
}

#[tauri::command]
pub async fn chat_delete_openapi_server(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::openapi::Server>, String> {
    let _ = secrets()?
        .delete(&crate::chat::openapi::secret_name(&id))
        .await;
    crate::chat::openapi::remove(&config_dir(&app)?, &id).map_err(String::from)
}

#[tauri::command]
pub fn chat_computer_settings(
    app: AppHandle,
) -> Result<crate::chat::computer::ComputerSettings, String> {
    Ok(crate::chat::computer::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_computer_settings(
    app: AppHandle,
    settings: crate::chat::computer::ComputerSettings,
) -> Result<(), String> {
    crate::chat::computer::save(&config_dir(&app)?, &settings).map_err(String::from)
}

#[tauri::command]
pub fn chat_custom_tools(
    app: AppHandle,
) -> Result<Vec<crate::chat::custom_tools::CustomTool>, String> {
    Ok(crate::chat::custom_tools::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_custom_tool(
    app: AppHandle,
    tool: crate::chat::custom_tools::CustomTool,
) -> Result<Vec<crate::chat::custom_tools::CustomTool>, String> {
    crate::chat::custom_tools::save(&config_dir(&app)?, &tool).map_err(String::from)
}

#[tauri::command]
pub fn chat_delete_custom_tool(
    app: AppHandle,
    name: String,
) -> Result<Vec<crate::chat::custom_tools::CustomTool>, String> {
    crate::chat::custom_tools::delete(&config_dir(&app)?, &name).map_err(String::from)
}

/// Runs a tool with sample input from its editor, in the sandbox.
#[tauri::command]
pub async fn chat_test_custom_tool(
    app: AppHandle,
    name: String,
    args: serde_json::Value,
) -> Result<String, String> {
    crate::chat::custom_tools::run(&config_dir(&app)?, "tool-tests", &name, &args)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub fn chat_get_privacy(app: AppHandle) -> Result<crate::chat::privacy::PrivacySettings, String> {
    Ok(crate::chat::privacy::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_privacy(
    app: AppHandle,
    settings: crate::chat::privacy::PrivacySettings,
) -> Result<(), String> {
    crate::chat::privacy::save(&config_dir(&app)?, &settings).map_err(String::from)
}
