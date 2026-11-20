//! Model providers, models, local models and usage.

use super::*;

#[tauri::command]
pub fn chat_list_providers(app: AppHandle) -> Result<Vec<ProviderView>, String> {
    Ok(providers::views(&providers::load(&config_dir(&app)?)))
}

#[tauri::command]
pub async fn chat_save_provider(
    app: AppHandle,
    provider: SaveProvider,
) -> Result<ProviderView, String> {
    if providers::info(&provider.vendor).is_none() {
        return Err(format!("unknown provider `{}`", provider.vendor));
    }
    let dir = config_dir(&app)?;
    let mut file = providers::load(&dir);
    let entry = file.providers.entry(provider.vendor.clone()).or_default();
    let name = providers::secret_name(&provider.vendor);
    if provider.clear_key {
        secrets()?.delete(&name).await?;
        entry.has_key = false;
    }
    if let Some(key) = provider
        .api_key
        .as_ref()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty())
    {
        secrets()?.store(&name, key).await?;
        entry.has_key = true;
    }
    entry.enabled = provider.enabled;
    entry.base_url = provider
        .base_url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let mut models: Vec<String> = provider
        .custom_models
        .into_iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    models.dedup();
    entry.custom_models = models;
    providers::save(&dir, &file)?;
    providers::views(&file)
        .into_iter()
        .find(|v| v.vendor == provider.vendor)
        .ok_or_else(|| "provider vanished".into())
}

#[tauri::command]
pub async fn chat_list_models(app: AppHandle) -> Result<ModelList, String> {
    let dir = config_dir(&app)?;
    let file = providers::load(&dir);
    let mut models = providers::models(&file).await;
    // ACP agents (Agent Client Protocol), installed or added by the user.
    models.extend(crate::chat::acp::models(&dir));
    if let Ok(root) = crate::chat::local_models::root() {
        models.extend(crate::chat::local_models::models(&root));
    }
    models.extend(crate::chat::functions::models(&dir));
    let default_model = file
        .default_model
        .clone()
        .filter(|d| models.iter().any(|m| &m.reference == d))
        .or_else(|| models.first().map(|m| m.reference.clone()));
    Ok(ModelList {
        models,
        default_model,
    })
}

#[tauri::command]
pub fn chat_set_default_model(app: AppHandle, reference: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut file = providers::load(&dir);
    file.default_model = Some(reference);
    providers::save(&dir, &file).map_err(String::from)
}

/// Sends one tiny request, so the settings page can confirm a key and base URL work.
#[tauri::command]
pub async fn chat_test_model(app: AppHandle, reference: String) -> Result<String, String> {
    let model = engine::connect_model(&config_dir(&app)?, &reference).await?;
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user("Reply with the single word: ready"),
    )
    .max_tokens(16u64);
    let response = tokio::time::timeout(std::time::Duration::from_secs(30), model.call(request))
        .await
        .map_err(|_| "The provider did not answer within 30 seconds.".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(response.text())
}

/// The model replies switch to when theirs fails.
#[tauri::command]
pub fn chat_backup_model(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::chat::providers::load(&config_dir(&app)?).backup_model)
}

#[tauri::command]
pub fn chat_set_backup_model(app: AppHandle, reference: Option<String>) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut file = crate::chat::providers::load(&dir);
    file.backup_model = reference.filter(|m| !m.trim().is_empty());
    crate::chat::providers::save(&dir, &file).map_err(String::from)
}

/// The model that reviews what the assistant writes for itself.
#[tauri::command]
pub fn chat_review_model(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::chat::providers::load(&config_dir(&app)?).review_model)
}

#[tauri::command]
pub fn chat_set_review_model(app: AppHandle, reference: Option<String>) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut file = crate::chat::providers::load(&dir);
    file.review_model = reference.filter(|m| !m.trim().is_empty());
    crate::chat::providers::save(&dir, &file).map_err(String::from)
}

/// The built-in models: which can be downloaded, and which are.
#[tauri::command]
pub fn chat_local_models() -> Result<Vec<crate::chat::local_models::View>, String> {
    Ok(crate::chat::local_models::list(
        &crate::chat::local_models::root()?,
    ))
}

/// Downloads a built-in model, reporting progress as `local-model-progress` events.
#[tauri::command]
pub async fn chat_download_local_model(app: AppHandle, id: String) -> Result<(), String> {
    use tauri::Emitter as _;
    let root = crate::chat::local_models::root()?;
    let emitter = app.clone();
    let model = id.clone();
    crate::chat::local_models::download(&root, &id, move |done, total| {
        let _ = emitter.emit(
            "local-model-progress",
            serde_json::json!({ "id": model, "done": done, "total": total }),
        );
    })
    .await
    .map_err(String::from)
}

#[tauri::command]
pub fn chat_delete_local_model(id: String) -> Result<(), String> {
    crate::chat::local_models::remove(&crate::chat::local_models::root()?, &id)
        .map_err(String::from)
}

/// The model settings every chat starts from.
#[tauri::command]
pub fn chat_get_defaults(app: AppHandle) -> Result<store::ChatParams, String> {
    Ok(store::chat_defaults(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_defaults(
    app: AppHandle,
    params: store::ChatParams,
) -> Result<store::ChatParams, String> {
    store::save_chat_defaults(&config_dir(&app)?, params).map_err(String::from)
}

#[tauri::command]
pub fn chat_usage_breakdown(app: AppHandle) -> Result<(Vec<UsageLine>, Vec<UsageLine>), String> {
    use chrono::Datelike;
    let dir = config_dir(&app)?;
    let now = chrono::Utc::now();
    let agents = crate::chat::agents::load(&dir);
    let mut by_model: std::collections::BTreeMap<String, UsageLine> = Default::default();
    let mut by_agent: std::collections::BTreeMap<String, UsageLine> = Default::default();
    for c in store::list_conversations(&dir)
        .into_iter()
        .filter(|c| c.updated_at.year() == now.year() && c.updated_at.month() == now.month())
    {
        let model = c.model.clone().unwrap_or_else(|| "unknown".into());
        let agent = c
            .agent_id
            .as_ref()
            .and_then(|id| agents.iter().find(|a| &a.id == id))
            .map_or_else(|| "Without an agent".to_string(), |a| a.name.clone());
        for (map, key) in [(&mut by_model, model), (&mut by_agent, agent)] {
            let line = map.entry(key.clone()).or_insert(UsageLine {
                name: key,
                chats: 0,
                tokens: 0,
                cost: 0.0,
            });
            line.chats += 1;
            line.tokens += c.tokens;
            line.cost += c.cost;
        }
    }
    let sorted = |m: std::collections::BTreeMap<String, UsageLine>| {
        let mut v: Vec<UsageLine> = m
            .into_values()
            .filter(|l| l.tokens > 0 || l.cost > 0.0)
            .collect();
        v.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(b.tokens.cmp(&a.tokens)));
        v
    };
    Ok((sorted(by_model), sorted(by_agent)))
}

#[tauri::command]
pub fn chat_cancel_local_model(id: String) {
    crate::chat::local_models::cancel(&id);
}
