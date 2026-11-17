//! IPC for the chat page. Every command is allowlisted in `permissions/chat.toml`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::engine::{self, TurnRequest};
use super::providers::{self, ModelView, ProviderView};
use super::store::{self, ConversationSummary, ConversationView, Instructions, Project};
use super::{ChatState, LOCAL_ACCOUNT};
use crate::account::AccountId;
use crate::tools::auth::keychain_secrets;

fn config_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    crate::settings::get_config_dir(app)
}

fn secrets() -> Result<crate::tools::auth::Secrets, String> {
    Ok(keychain_secrets(AccountId::parse(LOCAL_ACCOUNT)?))
}

#[tauri::command]
pub fn chat_list_providers(app: AppHandle) -> Result<Vec<ProviderView>, String> {
    Ok(providers::views(&providers::load(&config_dir(&app)?)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProvider {
    pub vendor: String,
    pub enabled: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub custom_models: Vec<String>,
    /// A new key; absent keeps the saved one.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_key: bool,
}

#[tauri::command]
pub async fn chat_save_provider(app: AppHandle, provider: SaveProvider) -> Result<ProviderView, String> {
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
    if let Some(key) = provider.api_key.as_ref().map(|k| k.trim()).filter(|k| !k.is_empty()) {
        secrets()?.store(&name, key).await?;
        entry.has_key = true;
    }
    entry.enabled = provider.enabled;
    entry.base_url = provider.base_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<ModelView>,
    pub default_model: Option<String>,
}

#[tauri::command]
pub async fn chat_list_models(app: AppHandle) -> Result<ModelList, String> {
    let file = providers::load(&config_dir(&app)?);
    let models = providers::models(&file).await;
    let default_model = file
        .default_model
        .clone()
        .filter(|d| models.iter().any(|m| &m.reference == d))
        .or_else(|| models.first().map(|m| m.reference.clone()));
    Ok(ModelList { models, default_model })
}

#[tauri::command]
pub fn chat_set_default_model(app: AppHandle, reference: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut file = providers::load(&dir);
    file.default_model = Some(reference);
    providers::save(&dir, &file)
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

#[tauri::command]
pub fn chat_list_conversations(app: AppHandle) -> Result<Vec<ConversationSummary>, String> {
    Ok(store::list_conversations(&config_dir(&app)?))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenConversation {
    #[serde(flatten)]
    pub view: ConversationView,
    pub running: bool,
}

#[tauri::command]
pub fn chat_get_conversation(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
) -> Result<OpenConversation, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    Ok(OpenConversation { view: store::view(&conversation), running: state.is_running(&id) })
}

#[tauri::command]
pub fn chat_send(app: AppHandle, state: State<'_, ChatState>, request: TurnRequest) -> Result<(), String> {
    if !store::valid_id(&request.conversation_id) {
        return Err("invalid conversation id".into());
    }
    if providers::split_reference(&request.model).is_none() {
        return Err("Choose a model first.".into());
    }
    let cancel = state
        .begin(&request.conversation_id)
        .ok_or("This conversation is already answering.")?;
    tauri::async_runtime::spawn(engine::run_turn(app, request, cancel));
    Ok(())
}

#[tauri::command]
pub fn chat_cancel(state: State<'_, ChatState>, conversation_id: String) -> bool {
    state.cancel(&conversation_id)
}

#[tauri::command]
pub fn chat_answer(state: State<'_, ChatState>, call_id: String, answer: String) -> bool {
    state.answer(&call_id, answer)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateConversation {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// `Some(None)` moves it out of its project.
    #[serde(default, deserialize_with = "serde_with_none::deserialize")]
    pub project_id: Option<Option<String>>,
    /// `(turn index, -1 | 0 | 1)`.
    #[serde(default)]
    pub rating: Option<(usize, i8)>,
}

/// `Option<Option<T>>` that tells a missing field from an explicit `null`.
mod serde_with_none {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Some(Option::deserialize(deserializer)?))
    }
}

#[tauri::command]
pub fn chat_update_conversation(app: AppHandle, update: UpdateConversation) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &update.id)?;
    if let Some(title) = update.title {
        conversation.title = title.trim().chars().take(120).collect();
    }
    if let Some(pinned) = update.pinned {
        conversation.pinned = pinned;
    }
    if let Some(project_id) = update.project_id {
        conversation.project_id = project_id;
    }
    if let Some((index, rating)) = update.rating {
        conversation.ratings.retain(|(i, _)| *i != index);
        if rating != 0 {
            conversation.ratings.push((index, rating.signum()));
        }
    }
    store::save_conversation(&dir, &conversation)
}

#[tauri::command]
pub fn chat_delete_conversation(app: AppHandle, state: State<'_, ChatState>, id: String) -> Result<(), String> {
    state.cancel(&id);
    store::delete_conversation(&config_dir(&app)?, &id)
}

#[tauri::command]
pub fn chat_list_projects(app: AppHandle) -> Result<Vec<Project>, String> {
    Ok(store::load_projects(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_project(app: AppHandle, project: Project) -> Result<Vec<Project>, String> {
    if !store::valid_id(&project.id) || project.name.trim().is_empty() {
        return Err("A project needs a name.".into());
    }
    let dir = config_dir(&app)?;
    let mut projects = store::load_projects(&dir);
    match projects.iter_mut().find(|p| p.id == project.id) {
        Some(existing) => *existing = project,
        None => projects.push(project),
    }
    store::save_projects(&dir, &projects)?;
    Ok(projects)
}

/// Deletes a project; its conversations stay, outside any project.
#[tauri::command]
pub fn chat_delete_project(app: AppHandle, id: String) -> Result<Vec<Project>, String> {
    let dir = config_dir(&app)?;
    let mut projects = store::load_projects(&dir);
    projects.retain(|p| p.id != id);
    store::save_projects(&dir, &projects)?;
    for summary in store::list_conversations(&dir) {
        if summary.project_id.as_deref() == Some(id.as_str()) {
            if let Ok(mut c) = store::load_conversation(&dir, &summary.id) {
                c.project_id = None;
                store::save_conversation(&dir, &c)?;
            }
        }
    }
    Ok(projects)
}

#[tauri::command]
pub fn chat_get_instructions(app: AppHandle) -> Result<Instructions, String> {
    Ok(store::load_instructions(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_instructions(app: AppHandle, instructions: Instructions) -> Result<(), String> {
    store::save_instructions(&config_dir(&app)?, &instructions)
}

/// A sample reply under unsaved instructions, for the instructions dialog's preview.
#[tauri::command]
pub async fn chat_preview_instructions(
    app: AppHandle,
    instructions: Instructions,
    model: String,
    prompt: String,
) -> Result<String, String> {
    let model = engine::connect_model(&config_dir(&app)?, &model).await?;
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .preamble(engine::preamble(
        &engine::Context { instructions: Some(&instructions), ..Default::default() },
        &[],
    ))
    .max_tokens(400u64);
    let response = tokio::time::timeout(std::time::Duration::from_secs(60), model.call(request))
        .await
        .map_err(|_| "The provider did not answer in time.".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(response.text())
}

/// Writes a conversation as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_conversation(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    let mut text = format!("# {}\n\n", engine::fallback_title(&conversation));
    if !conversation.title.is_empty() {
        text = format!("# {}\n\n", conversation.title);
    }
    for turn in store::turns(&conversation) {
        text.push_str(if turn.role == "user" { "## You\n\n" } else { "## Assistant\n\n" });
        for part in turn.parts {
            match part {
                store::Part::Text { text: t } => text.push_str(&format!("{t}\n\n")),
                store::Part::Tool { name, .. } => text.push_str(&format!("_Used {name}_\n\n")),
                store::Part::Reasoning { .. } => {}
            }
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{}.md", sanitize_file_name(&conversation.title)))
        .add_filter("Markdown", &["md"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(true)
}

fn sanitize_file_name(title: &str) -> String {
    let name: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
        .collect();
    let name = name.trim();
    if name.is_empty() { "conversation".into() } else { name.chars().take(80).collect() }
}

#[tauri::command]
pub fn chat_get_memories(app: AppHandle) -> Result<super::memory::Memories, String> {
    Ok(super::memory::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_add_memory(app: AppHandle, text: String) -> Result<super::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = super::memory::load(&dir);
    super::memory::add(&mut memories, &text, "user")?;
    super::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_update_memory(app: AppHandle, id: String, text: String) -> Result<super::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = super::memory::load(&dir);
    let text = text.trim();
    if text.is_empty() || text.chars().count() > super::memory::MAX_LENGTH {
        return Err(format!("A memory is 1 to {} characters.", super::memory::MAX_LENGTH));
    }
    let memory = memories.items.iter_mut().find(|m| m.id == id).ok_or("No such memory.")?;
    memory.text = text.to_string();
    super::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_delete_memory(app: AppHandle, id: Option<String>) -> Result<super::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = super::memory::load(&dir);
    match id {
        Some(id) => {
            super::memory::remove(&mut memories, &id);
        }
        None => memories.items.clear(),
    }
    super::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_set_memory_enabled(app: AppHandle, enabled: bool) -> Result<super::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = super::memory::load(&dir);
    memories.enabled = enabled;
    super::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_list_skills(app: AppHandle) -> Result<Vec<super::skills::Skill>, String> {
    Ok(super::skills::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_skill(app: AppHandle, name: String, description: String, body: String) -> Result<Vec<super::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    super::skills::save(&dir, name.trim(), &description, &body)?;
    Ok(super::skills::list(&dir))
}

#[tauri::command]
pub fn chat_delete_skill(app: AppHandle, name: String) -> Result<Vec<super::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    super::skills::delete(&dir, &name)?;
    Ok(super::skills::list(&dir))
}

#[tauri::command]
pub fn chat_set_skill_enabled(app: AppHandle, name: String, enabled: bool) -> Result<Vec<super::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    super::skills::set_enabled(&dir, &name, enabled)?;
    Ok(super::skills::list(&dir))
}

/// Imports a `SKILL.md` the user picks; returns the skill's name, or `None` when cancelled.
#[tauri::command]
pub async fn chat_import_skill(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import a skill (SKILL.md)")
        .add_filter("Skill", &["md"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    super::skills::import(&config_dir(&app)?, &path).map(Some)
}

/// Manages chat state at startup.
pub fn setup(app: &tauri::App) {
    app.manage(ChatState::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_distinguishes_null_from_missing_project() {
        let missing: UpdateConversation = serde_json::from_str(r#"{"id":"a"}"#).unwrap();
        assert_eq!(missing.project_id, None);
        let cleared: UpdateConversation = serde_json::from_str(r#"{"id":"a","projectId":null}"#).unwrap();
        assert_eq!(cleared.project_id, Some(None));
        let set: UpdateConversation = serde_json::from_str(r#"{"id":"a","projectId":"p"}"#).unwrap();
        assert_eq!(set.project_id, Some(Some("p".into())));
    }

    #[test]
    fn export_names_are_safe() {
        assert_eq!(sanitize_file_name("a/b: c?"), "a_b_ c_");
        assert_eq!(sanitize_file_name("   "), "conversation");
    }
}
