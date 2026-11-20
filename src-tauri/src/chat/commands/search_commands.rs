//! Search: every chat, library and the web from one box, answers from the results, the web search engine's settings, and web sources read as articles or shown by their icons.

use super::*;

/// Full-text search over every conversation's title and messages.
#[tauri::command]
pub async fn chat_search(
    app: AppHandle,
    query: String,
) -> Result<Vec<crate::chat::index::Hit>, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::index::search(&dir, &query, 40))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_search_libraries(
    app: AppHandle,
    query: String,
    library_id: Option<String>,
) -> Result<Vec<crate::chat::libraries::Passage>, String> {
    let only: Vec<String> = library_id.into_iter().collect();
    Ok(
        crate::chat::embeddings::search(&config_dir(&app)?, &only, &query, 8)
            .await
            .0,
    )
}

/// Search mode: one query over libraries, knowledge pages, chats and (when asked) the web.
#[tauri::command]
pub async fn chat_search_everything(
    app: AppHandle,
    query: String,
    web: bool,
) -> Result<crate::chat::search_all::Results, String> {
    Ok(crate::chat::search_all::search(&config_dir(&app)?, &query, web).await)
}

/// A short answer from search results, citing them by number.
#[tauri::command]
pub async fn chat_answer_from_results(
    app: AppHandle,
    model: String,
    query: String,
    cards: Vec<crate::chat::search_all::Card>,
) -> Result<String, String> {
    if cards.is_empty() {
        return Err("There is nothing to answer from.".into());
    }
    let prompt = crate::chat::search_all::answer_prompt(&query, &cards);
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(4_000u64);
    engine::ask_text(&config_dir(&app)?, &model, request, 90).await
}

#[tauri::command]
pub fn chat_search_settings(app: AppHandle) -> Result<crate::chat::search::SearchSettings, String> {
    Ok(crate::chat::search::load_settings(&config_dir(&app)?))
}

/// Saves the web search engine; a key given is stored in the keychain.
#[tauri::command]
pub async fn chat_save_search_settings(
    app: AppHandle,
    settings: crate::chat::search::SearchSettings,
    key: Option<String>,
) -> Result<crate::chat::search::SearchSettings, String> {
    let dir = config_dir(&app)?;
    let mut settings = settings;
    settings.has_key = crate::chat::search::load_settings(&dir).has_key;
    if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        secrets()?.store(crate::chat::search::SECRET, &key).await?;
        settings.has_key = true;
    }
    crate::chat::search::save_settings(&dir, &settings)?;
    Ok(crate::chat::search::load_settings(&dir))
}

/// A source read as an article, for the reader in the side panel.
#[tauri::command]
pub async fn chat_read_article(url: String) -> Result<crate::chat::reader::Article, String> {
    crate::chat::reader::read(&url).await
}

/// A site's icon as a `data:` URL, for sources and citations; `None` when it has none.
#[tauri::command]
pub async fn chat_favicon(app: AppHandle, host: String) -> Result<Option<String>, String> {
    Ok(crate::chat::favicon::get(&config_dir(&app)?, &host).await)
}
