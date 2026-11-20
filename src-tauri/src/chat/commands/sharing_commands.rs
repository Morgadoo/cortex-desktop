//! Share links: a conversation or an app served at a link on this computer's network, with an expiry.

use super::*;

/// Shares a page (a conversation, an app) at a link on this computer's network.
#[tauri::command]
pub async fn chat_share_link(
    app: AppHandle,
    title: String,
    html: String,
    is_app: Option<bool>,
    expires_hours: Option<u32>,
) -> Result<crate::chat::share::SharedLink, String> {
    crate::chat::share::share(
        &crate::chat::share::server(&app),
        &config_dir(&app)?,
        &title,
        &html,
        is_app.unwrap_or(false),
        expires_hours,
    )
    .await
}

#[tauri::command]
pub fn chat_set_share_expiry(
    app: AppHandle,
    token: String,
    hours: Option<u32>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    crate::chat::share::set_expiry(&config_dir(&app)?, &token, hours)
}

#[tauri::command]
pub async fn chat_shared_links(
    app: AppHandle,
) -> Result<Vec<crate::chat::share::SharedLink>, String> {
    Ok(crate::chat::share::links(&crate::chat::share::server(&app), &config_dir(&app)?).await)
}

#[tauri::command]
pub fn chat_stop_share(app: AppHandle, token: String) -> Result<(), String> {
    crate::chat::share::stop(&config_dir(&app)?, &token)
}
