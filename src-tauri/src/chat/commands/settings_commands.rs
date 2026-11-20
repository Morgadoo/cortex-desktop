//! App-wide settings: sync, backups, encryption, storage health, the activity log, document extraction, the vector store, the local API, chat apps and the interface.

use super::*;

/// Telegram settings, with whether a token is saved.
#[tauri::command]
pub async fn chat_telegram(app: AppHandle) -> Result<crate::chat::telegram::Settings, String> {
    let mut s = crate::chat::telegram::load(&config_dir(&app)?);
    s.has_token = crate::chat::telegram::token().await.is_some();
    Ok(s)
}

/// Saves a bot token after checking it with Telegram; empty removes it.
#[tauri::command]
pub async fn chat_set_telegram_token(
    app: AppHandle,
    token: String,
) -> Result<crate::chat::telegram::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::telegram::load(&dir);
    if token.trim().is_empty() {
        crate::chat::telegram::set_token("").await?;
        s.enabled = false;
        s.bot_name = None;
    } else {
        s.bot_name = Some(crate::chat::telegram::check(token.trim()).await?);
        crate::chat::telegram::set_token(&token).await?;
    }
    crate::chat::telegram::save(&dir, &s)?;
    chat_telegram(app).await
}

/// Changes the Telegram settings: on/off, agent, changes allowed; opens pairing or unpairs.
#[tauri::command]
pub async fn chat_save_telegram(
    app: AppHandle,
    enabled: bool,
    agent_id: Option<String>,
    allow_changes: bool,
    open_pairing: bool,
    unpair: Option<i64>,
) -> Result<crate::chat::telegram::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::telegram::load(&dir);
    s.enabled = enabled;
    s.agent_id = agent_id.filter(|a| !a.is_empty());
    s.allow_changes = allow_changes;
    if open_pairing {
        let mut b = [0u8; 4];
        rand::fill(&mut b);
        s.pairing_code = Some(format!("{:06}", u32::from_le_bytes(b) % 1_000_000));
    }
    if let Some(id) = unpair {
        if let Some(i) = s.paired.iter().position(|p| *p == id) {
            s.paired.remove(i);
            if i < s.paired_names.len() {
                s.paired_names.remove(i);
            }
        }
    }
    crate::chat::telegram::save(&dir, &s)?;
    if s.enabled {
        crate::chat::telegram::start(&app);
    }
    chat_telegram(app).await
}

#[tauri::command]
pub async fn chat_discord(app: AppHandle) -> Result<crate::chat::discord::Settings, String> {
    let mut s = crate::chat::discord::load(&config_dir(&app)?);
    s.has_token = crate::chat::discord::token().await.is_some();
    Ok(s)
}

/// Saves the bot token after checking it with Discord; empty removes it.
#[tauri::command]
pub async fn chat_set_discord_token(
    app: AppHandle,
    token: String,
) -> Result<crate::chat::discord::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::discord::load(&dir);
    if token.trim().is_empty() {
        crate::chat::discord::set_token("").await?;
        s.enabled = false;
        s.bot_name = None;
    } else {
        s.bot_name = Some(crate::chat::discord::check(token.trim()).await?);
        crate::chat::discord::set_token(&token).await?;
    }
    crate::chat::discord::save(&dir, &s)?;
    chat_discord(app).await
}

#[tauri::command]
pub async fn chat_save_discord(
    app: AppHandle,
    enabled: bool,
    agent_id: Option<String>,
    allow_changes: bool,
    open_pairing: bool,
    unpair: Option<String>,
) -> Result<crate::chat::discord::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::discord::load(&dir);
    s.enabled = enabled;
    s.agent_id = agent_id.filter(|a| !a.is_empty());
    s.allow_changes = allow_changes;
    if open_pairing {
        let mut b = [0u8; 4];
        rand::fill(&mut b);
        s.pairing_code = Some(format!("{:06}", u32::from_le_bytes(b) % 1_000_000));
    }
    if let Some(id) = unpair {
        s.paired.retain(|p| *p != id);
    }
    crate::chat::discord::save(&dir, &s)?;
    if s.enabled {
        crate::chat::discord::start(&app);
    }
    chat_discord(app).await
}

#[tauri::command]
pub async fn chat_slack(app: AppHandle) -> Result<crate::chat::slack::Settings, String> {
    let mut s = crate::chat::slack::load(&config_dir(&app)?);
    s.has_tokens = crate::chat::slack::tokens().await.is_some();
    Ok(s)
}

/// Saves the app-level and bot tokens after checking them with Slack; empty removes them.
#[tauri::command]
pub async fn chat_set_slack_tokens(
    app: AppHandle,
    app_token: String,
    bot_token: String,
) -> Result<crate::chat::slack::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::slack::load(&dir);
    if app_token.trim().is_empty() && bot_token.trim().is_empty() {
        crate::chat::slack::set_tokens("", "").await?;
        s.enabled = false;
        s.team = None;
    } else {
        s.team = Some(crate::chat::slack::check(app_token.trim(), bot_token.trim()).await?);
        crate::chat::slack::set_tokens(&app_token, &bot_token).await?;
    }
    crate::chat::slack::save(&dir, &s)?;
    chat_slack(app).await
}

#[tauri::command]
pub async fn chat_save_slack(
    app: AppHandle,
    enabled: bool,
    agent_id: Option<String>,
    allow_changes: bool,
    open_pairing: bool,
    unpair: Option<String>,
) -> Result<crate::chat::slack::Settings, String> {
    let dir = config_dir(&app)?;
    let mut s = crate::chat::slack::load(&dir);
    s.enabled = enabled;
    s.agent_id = agent_id.filter(|a| !a.is_empty());
    s.allow_changes = allow_changes;
    if open_pairing {
        let mut b = [0u8; 4];
        rand::fill(&mut b);
        s.pairing_code = Some(format!("{:06}", u32::from_le_bytes(b) % 1_000_000));
    }
    if let Some(id) = unpair {
        s.paired.retain(|p| *p != id);
    }
    crate::chat::slack::save(&dir, &s)?;
    if s.enabled {
        crate::chat::slack::start(&app);
    }
    chat_slack(app).await
}

/// The interface languages on offer, as (code, name).
#[tauri::command]
pub fn chat_ui_languages() -> Vec<(String, String)> {
    crate::chat::ui_language::LANGUAGES
        .iter()
        .map(|(c, n)| (c.to_string(), n.to_string()))
        .collect()
}

#[tauri::command]
pub fn chat_reset_ui_translations(app: AppHandle, code: String) -> Result<(), String> {
    crate::chat::ui_language::reset(&config_dir(&app)?, &code).map_err(String::from)
}

/// Encryption at rest: whether it is on, and how many files are sealed.
#[tauri::command]
pub async fn chat_encryption_status(
    app: AppHandle,
) -> Result<crate::chat::at_rest::Status, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::at_rest::status(&dir))
        .await
        .map_err(|e| e.to_string())
}

/// Turns encryption at rest on or off; returns how many files were converted.
#[tauri::command]
pub async fn chat_set_encryption(app: AppHandle, on: bool) -> Result<usize, String> {
    let dir = config_dir(&app)?;
    if on {
        crate::chat::at_rest::enable(&dir)
            .await
            .map_err(String::from)
    } else {
        crate::chat::at_rest::disable(&dir)
            .await
            .map_err(String::from)
    }
}

#[tauri::command]
pub async fn chat_sync_settings(app: AppHandle) -> Result<SyncView, String> {
    let settings = crate::chat::sync::settings(&config_dir(&app)?);
    Ok(SyncView {
        settings,
        has_passphrase: crate::chat::sync::passphrase()
            .await
            .ok()
            .flatten()
            .is_some(),
    })
}

/// Sets the sync folder, interval and passphrase; checks the passphrase against the folder
/// (another device may have set it up) before saving. `folder: None` turns sync off.
#[tauri::command]
pub async fn chat_save_sync_settings(
    app: AppHandle,
    folder: Option<String>,
    every_minutes: u32,
    passphrase: Option<String>,
) -> Result<SyncView, String> {
    let dir = config_dir(&app)?;
    let mut settings = crate::chat::sync::settings(&dir);
    let Some(folder) = folder.filter(|f| !f.trim().is_empty()) else {
        settings.folder = None;
        crate::chat::sync::save_settings(&dir, &settings)?;
        crate::chat::sync::set_passphrase(None).await?;
        return chat_sync_settings(app).await;
    };
    let phrase = match passphrase.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => crate::chat::sync::passphrase()
            .await?
            .ok_or("Set a passphrase: it encrypts everything in the sync folder.")?,
    };
    let remote = crate::chat::sync::remote_dir(&folder);
    let check = phrase.clone();
    tokio::task::spawn_blocking(move || crate::chat::sync::unlock(&remote, &check))
        .await
        .map_err(|e| e.to_string())??;
    crate::chat::sync::set_passphrase(Some(&phrase)).await?;
    settings.folder = Some(folder);
    settings.every_minutes = every_minutes.min(24 * 60);
    crate::chat::sync::save_settings(&dir, &settings)?;
    chat_sync_settings(app).await
}

/// Syncs now; returns what happened, in words.
#[tauri::command]
pub async fn chat_sync_now(app: AppHandle) -> Result<String, String> {
    let report = crate::chat::sync::run(
        &app.state::<ChatState>()
            .part::<crate::chat::sync::Syncing>(),
        &config_dir(&app)?,
    )
    .await?;
    Ok(report.summary())
}

/// What the app may send about itself, and whether this build can send error reports at all.
#[tauri::command]
pub fn chat_diagnostics_prefs(app: AppHandle) -> Result<serde_json::Value, String> {
    let dir = crate::settings::get_config_dir(&app)?;
    Ok(
        serde_json::json!({ "prefs": crate::reporting::load_prefs(&dir), "reportingBuilt": crate::reporting::compiled_in() }),
    )
}

#[tauri::command]
pub fn chat_set_diagnostics_prefs(
    app: AppHandle,
    prefs: crate::reporting::DiagnosticsPrefs,
) -> Result<(), String> {
    crate::reporting::save_prefs(&crate::settings::get_config_dir(&app)?, &prefs)
}

/// The local API's latest requests, newest first.
#[tauri::command]
pub fn chat_api_server_log(
    server: State<'_, crate::chat::api_server::ApiServer>,
) -> Vec<crate::chat::api_server::LogEntry> {
    crate::chat::api_server::recent(&server)
}

#[tauri::command]
pub async fn chat_api_server(app: AppHandle) -> Result<ApiServerView, String> {
    api_server_view(&app).await
}

/// Saves the local API's settings and starts, restarts or stops it to match.
#[tauri::command]
pub async fn chat_save_api_server(
    app: AppHandle,
    settings: crate::chat::api_server::ApiServerSettings,
    regenerate_token: bool,
    regenerate_widget_token: Option<bool>,
) -> Result<ApiServerView, String> {
    crate::chat::api_server::save(&config_dir(&app)?, &settings)?;
    if regenerate_token {
        crate::chat::api_server::token(true).await?;
    }
    if regenerate_widget_token == Some(true) {
        crate::chat::api_server::widget_token(true).await?;
    }
    crate::chat::api_server::apply(&app).await;
    api_server_view(&app).await
}

/// The languages code blocks can run in, for their Run buttons.
#[tauri::command]
pub fn chat_snippet_languages() -> Vec<&'static str> {
    crate::chat::sandbox::languages()
}

/// Deleted conversations, kept for 30 days.
#[tauri::command]
pub fn chat_trash(app: AppHandle) -> Result<Vec<crate::chat::backup::TrashItem>, String> {
    Ok(crate::chat::backup::list_trash(&config_dir(&app)?))
}

/// The activity log: how many entries, and since when.
#[tauri::command]
pub fn chat_audit_summary(app: AppHandle) -> Result<serde_json::Value, String> {
    let all = crate::chat::audit::entries(&config_dir(&app)?, None);
    Ok(serde_json::json!({
        "count": all.len(),
        "since": all.first().map(|e| e.at),
        "approvals": all.iter().filter(|e| e.kind == "approval").count(),
        "denied": all.iter().filter(|e| e.outcome == "denied").count(),
        "errors": all.iter().filter(|e| e.outcome == "error").count(),
    }))
}

#[tauri::command]
pub fn chat_clear_audit(app: AppHandle) -> Result<(), String> {
    crate::chat::audit::clear(&config_dir(&app)?).map_err(String::from)
}

/// The document extraction engine (built-in, Tika or Docling); `change` sets it.
#[tauri::command]
pub fn chat_extraction(
    app: AppHandle,
    change: Option<crate::chat::extraction::Extraction>,
) -> Result<crate::chat::extraction::Extraction, String> {
    let dir = config_dir(&app)?;
    if let Some(e) = change {
        crate::chat::extraction::save(&dir, &e)?;
    }
    Ok(crate::chat::extraction::load(&dir))
}

/// Checks the Qdrant server in the saved settings; returns its version.
#[tauri::command]
pub async fn chat_check_vector_store(app: AppHandle) -> Result<String, String> {
    let settings = crate::chat::embeddings::load(&config_dir(&app)?);
    if settings.qdrant_url.is_empty() {
        return Err("Enter the Qdrant server's address first.".into());
    }
    crate::chat::embeddings::qdrant::check(&settings)
        .await
        .map_err(String::from)
}

/// How the stored data is doing: counts, size, backups, and files that cannot be read.
#[tauri::command]
pub async fn chat_storage_health(app: AppHandle) -> Result<crate::chat::backup::Health, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::backup::health(&dir))
        .await
        .map_err(|e| e.to_string())
}
