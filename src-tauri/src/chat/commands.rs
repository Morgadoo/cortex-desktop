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
    #[serde(default)]
    pub unread: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
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
    if let Some(unread) = update.unread {
        conversation.unread = unread;
    }
    if let Some(tags) = update.tags {
        conversation.tags = store::clean_tags(&tags);
    }
    if let Some(archived) = update.archived {
        conversation.archived = archived;
        if archived {
            conversation.pinned = false;
        }
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
pub fn chat_workflows(app: AppHandle) -> Result<Vec<super::workflows::Workflow>, String> {
    Ok(super::workflows::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_workflow(app: AppHandle, workflow: super::workflows::Workflow) -> Result<Vec<super::workflows::Workflow>, String> {
    super::workflows::save(&config_dir(&app)?, workflow)
}

#[tauri::command]
pub fn chat_delete_workflow(app: AppHandle, id: String) -> Result<Vec<super::workflows::Workflow>, String> {
    super::workflows::delete(&config_dir(&app)?, &id)
}

/// Whether code runs without asking each time.
#[tauri::command]
pub fn chat_code_runs_allowed(app: AppHandle) -> Result<bool, String> {
    Ok(super::sandbox::always_allowed(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_code_runs_allowed(app: AppHandle, allowed: bool) -> Result<(), String> {
    super::sandbox::set_always_allowed(&config_dir(&app)?, allowed)
}

/// The working tree's changes in a coding session's folder.
#[tauri::command]
pub async fn chat_code_changes(app: AppHandle, conversation_id: String) -> Result<super::code::Changes, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || super::code::changes(&dir)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn chat_media_models(app: AppHandle) -> Result<super::media::MediaModels, String> {
    Ok(super::media::models(&config_dir(&app)?))
}

/// Chooses the image, speech and voice models; an empty value goes back to the default.
#[tauri::command]
pub fn chat_set_media_models(
    app: AppHandle,
    image_model: Option<String>,
    speech_model: Option<String>,
    voice_model: Option<String>,
) -> Result<super::media::MediaModels, String> {
    let dir = config_dir(&app)?;
    let mut file = super::providers::load(&dir);
    file.image_model = image_model.filter(|m| !m.trim().is_empty());
    file.speech_model = speech_model.filter(|m| !m.trim().is_empty());
    file.voice_model = voice_model.filter(|m| !m.trim().is_empty());
    super::providers::save(&dir, &file)?;
    Ok(super::media::models(&dir))
}

/// Transcribes speech recorded in the composer (base64 audio).
#[tauri::command]
pub async fn chat_transcribe(app: AppHandle, audio: String, mime: String) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio.as_bytes()).map_err(|e| e.to_string())?;
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("That recording is too long.".into());
    }
    let ext = match mime.split(';').next().unwrap_or_default() {
        "audio/ogg" => "ogg",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" => "mp3",
        _ => "webm",
    };
    super::media::transcribe(&config_dir(&app)?, bytes, &format!("speech.{ext}")).await
}

/// Reads text aloud; returns base64 audio and its media type.
#[tauri::command]
pub async fn chat_speak(app: AppHandle, text: String) -> Result<(String, String), String> {
    use base64::Engine as _;
    let (audio, mime) = super::media::speak(&config_dir(&app)?, &text).await?;
    Ok((base64::engine::general_purpose::STANDARD.encode(audio), mime))
}

#[tauri::command]
pub fn chat_knowledge_pages(app: AppHandle) -> Result<Vec<super::knowledge::Page>, String> {
    Ok(super::knowledge::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_knowledge_page(app: AppHandle, id: Option<String>, title: String, body: String) -> Result<Vec<super::knowledge::Page>, String> {
    let dir = config_dir(&app)?;
    super::knowledge::write(&dir, id.as_deref(), &title, &body, "user", &[])?;
    Ok(super::knowledge::load(&dir))
}

#[tauri::command]
pub fn chat_delete_knowledge_page(app: AppHandle, id: String) -> Result<Vec<super::knowledge::Page>, String> {
    super::knowledge::delete(&config_dir(&app)?, &id)
}

/// Reads an uploaded workbook (base64) into CSV text per sheet.
#[tauri::command]
pub fn chat_read_spreadsheet(data: String) -> Result<Vec<(String, String)>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
    if bytes.len() > 30 * 1024 * 1024 {
        return Err("That workbook is too large.".into());
    }
    super::sheets::read_workbook(&bytes)
}

/// Adds guidance to a running turn; it joins the conversation at the turn's next step.
#[tauri::command]
pub fn chat_steer(state: State<'_, ChatState>, conversation_id: String, text: String) -> Result<bool, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(false);
    }
    Ok(state.steer(&conversation_id, text))
}

/// Commits the working tree of a coding session's folder.
#[tauri::command]
pub async fn chat_code_commit(app: AppHandle, conversation_id: String, message: String) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || super::code::commit(&dir, &message)).await.map_err(|e| e.to_string())?
}

/// Pushes a coding session's branch and opens a pull request for it.
#[tauri::command]
pub async fn chat_code_pull_request(app: AppHandle, conversation_id: String, title: String) -> Result<super::code::PullRequest, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    if title.trim().is_empty() {
        return Err("Give the pull request a title.".into());
    }
    tokio::task::spawn_blocking(move || super::code::open_pull_request(&dir, &title)).await.map_err(|e| e.to_string())?
}

/// Shows another answer to the last prompt.
#[tauri::command]
pub fn chat_select_version(app: AppHandle, state: State<'_, ChatState>, id: String, from: usize, index: usize) -> Result<(), String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::select_version(&mut conversation, from, index)?;
    store::save_conversation(&dir, &conversation)
}

/// Imports a ChatGPT or Claude data export (the zip, or its conversations.json).
#[tauri::command]
pub async fn chat_import_conversations(app: AppHandle) -> Result<Option<super::import::ImportReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import a ChatGPT or Claude export")
        .add_filter("Export", &["zip", "json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || super::import::import(&dir, &path)).await.map_err(|e| e.to_string())?.map(Some)
}

/// Keeps a temporary conversation, such as the answer chosen in a model comparison.
#[tauri::command]
pub fn chat_keep_conversation(app: AppHandle, id: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    conversation.temporary = false;
    store::save_conversation(&dir, &conversation)
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
    let text = conversation_markdown(&conversation);
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

#[tauri::command]
pub fn chat_get_canvas(app: AppHandle, id: String) -> Result<super::canvas::Canvas, String> {
    super::canvas::load(&config_dir(&app)?, &id)
}

/// Saves the user's edit as a new version.
#[tauri::command]
pub fn chat_save_canvas(app: AppHandle, id: String, title: Option<String>, content: String) -> Result<super::canvas::Canvas, String> {
    let dir = config_dir(&app)?;
    let existing = super::canvas::load(&dir, &id)?;
    super::canvas::write(&dir, &existing.conversation_id, Some(&id), title.as_deref(), &content, "user")
}

/// Writes a canvas as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_canvas(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let canvas = super::canvas::load(&config_dir(&app)?, &id)?;
    let sheet = canvas.kind == "sheet";
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    let dialog = if sheet {
        dialog
            .set_file_name(format!("{}.xlsx", sanitize_file_name(&canvas.title)))
            .add_filter("Excel workbook", &["xlsx"])
            .add_filter("CSV", &["csv"])
    } else {
        dialog.set_file_name(format!("{}.md", sanitize_file_name(&canvas.title))).add_filter("Markdown", &["md"])
    };
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    if sheet {
        let csv = super::sheets::sheet_csv(canvas.content());
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
            std::fs::write(&path, csv).map_err(|e| e.to_string())?;
        } else {
            super::sheets::write_xlsx(&path.with_extension("xlsx"), &canvas.title, &super::sheets::parse_csv(&csv))?;
        }
        return Ok(true);
    }
    std::fs::write(path, canvas.content()).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn chat_list_agents(app: AppHandle) -> Result<Vec<super::agents::Agent>, String> {
    Ok(super::agents::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_agent_templates() -> Vec<super::agents::Agent> {
    super::agents::templates()
}

#[tauri::command]
pub fn chat_save_agent(app: AppHandle, agent: super::agents::Agent) -> Result<Vec<super::agents::Agent>, String> {
    super::agents::save(&config_dir(&app)?, agent)
}

#[tauri::command]
pub fn chat_delete_agent(app: AppHandle, id: String) -> Result<Vec<super::agents::Agent>, String> {
    super::agents::delete(&config_dir(&app)?, &id)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    #[serde(flatten)]
    pub task: super::tasks::Task,
    pub schedule: String,
}

fn task_views(tasks: Vec<super::tasks::Task>) -> Vec<TaskView> {
    tasks
        .into_iter()
        .map(|task| TaskView { schedule: super::tasks::describe(&task.frequency), task })
        .collect()
}

#[tauri::command]
pub fn chat_list_tasks(app: AppHandle) -> Result<Vec<TaskView>, String> {
    Ok(task_views(super::tasks::load(&config_dir(&app)?)))
}

#[tauri::command]
pub fn chat_save_task(app: AppHandle, task: super::tasks::Task) -> Result<Vec<TaskView>, String> {
    super::tasks::save(&config_dir(&app)?, task).map(task_views)
}

#[tauri::command]
pub fn chat_delete_task(app: AppHandle, id: String) -> Result<Vec<TaskView>, String> {
    super::tasks::delete(&config_dir(&app)?, &id).map(task_views)
}

#[tauri::command]
pub fn chat_run_task(app: AppHandle, id: String) -> Result<(), String> {
    super::tasks::run_now(&app, &id)
}

#[tauri::command]
pub fn chat_list_libraries(app: AppHandle) -> Result<Vec<super::libraries::Library>, String> {
    Ok(super::libraries::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_create_library(app: AppHandle, name: String, description: String) -> Result<super::libraries::Library, String> {
    super::libraries::create(&config_dir(&app)?, &name, &description)
}

#[tauri::command]
pub fn chat_delete_library(app: AppHandle, id: String) -> Result<(), String> {
    super::libraries::delete(&config_dir(&app)?, &id)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    pub library: super::libraries::Library,
    pub errors: Vec<String>,
}

/// Asks for files and adds each to the library, reporting the ones that could not be read.
#[tauri::command]
pub async fn chat_add_documents(app: AppHandle, id: String) -> Result<Option<AddResult>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Add documents")
        .add_filter("Documents", &["pdf", "md", "txt", "html", "htm", "csv", "json", "yaml", "yml", "xml", "rs", "py", "js", "ts", "go", "java", "c", "cpp", "h", "sql", "log"])
        .pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    let Some(paths) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let dir = config_dir(&app)?;
    let mut errors = Vec::new();
    for path in paths {
        let Ok(path) = path.into_path() else { continue };
        if let Err(e) = tokio::task::spawn_blocking({
            let dir = dir.clone();
            let id = id.clone();
            move || super::libraries::add(&dir, &id, &path)
        })
        .await
        .map_err(|e| e.to_string())?
        {
            errors.push(e);
        }
    }
    Ok(Some(AddResult { library: super::libraries::load(&dir, &id)?, errors }))
}

#[tauri::command]
pub fn chat_remove_document(app: AppHandle, library_id: String, document_id: String) -> Result<super::libraries::Library, String> {
    super::libraries::remove(&config_dir(&app)?, &library_id, &document_id)
}

#[tauri::command]
pub fn chat_search_libraries(app: AppHandle, query: String, library_id: Option<String>) -> Result<Vec<super::libraries::Passage>, String> {
    let only: Vec<String> = library_id.into_iter().collect();
    Ok(super::libraries::search(&config_dir(&app)?, &only, &query, 8))
}

#[tauri::command]
pub fn chat_get_privacy(app: AppHandle) -> Result<super::privacy::PrivacySettings, String> {
    Ok(super::privacy::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_privacy(app: AppHandle, settings: super::privacy::PrivacySettings) -> Result<(), String> {
    super::privacy::save(&config_dir(&app)?, &settings)
}

/// The project's file library, created on first use.
#[tauri::command]
pub fn chat_project_library(app: AppHandle, project_id: String) -> Result<super::libraries::Library, String> {
    let dir = config_dir(&app)?;
    let mut projects = store::load_projects(&dir);
    let project = projects.iter_mut().find(|p| p.id == project_id).ok_or("No such project.")?;
    if let Some(id) = &project.library_id {
        if let Ok(library) = super::libraries::load(&dir, id) {
            return Ok(library);
        }
    }
    let library = super::libraries::create(&dir, &format!("{} files", project.name), "Files of the project")?;
    project.library_id = Some(library.id.clone());
    store::save_projects(&dir, &projects)?;
    Ok(library)
}

/// Exports everything to a folder the user picks: each conversation as Markdown, and a copy of
/// the app's data (conversations, projects, agents, tasks, knowledge, skills, libraries).
#[tauri::command]
pub async fn chat_export_all(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Export to folder").pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let Some(target) = rx.await.map_err(|e| e.to_string())? else { return Ok(None) };
    let target = target.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    let out = target.join(format!("lumo-export-{}", chrono::Local::now().format("%Y-%m-%d-%H%M")));
    let markdown = out.join("conversations");
    std::fs::create_dir_all(&markdown).map_err(|e| e.to_string())?;
    let mut count = 0;
    for summary in store::list_conversations(&dir) {
        let Ok(conversation) = store::load_conversation(&dir, &summary.id) else { continue };
        let name = format!("{} {}.md", summary.updated_at.format("%Y-%m-%d"), sanitize_file_name(&summary.title));
        std::fs::write(markdown.join(name), conversation_markdown(&conversation)).map_err(|e| e.to_string())?;
        count += 1;
    }
    copy_dir(&store::chat_dir(&dir), &out.join("data/chat"))?;
    copy_dir(&super::skills::skills_dir(&dir), &out.join("data/skills"))?;
    Ok(Some(format!("Exported {count} conversations to {}", out.display())))
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(from).into_iter().filter_map(Result::ok) {
        let rel = entry.path().strip_prefix(from).map_err(|e| e.to_string())?;
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn conversation_markdown(conversation: &store::Conversation) -> String {
    let title = if conversation.title.is_empty() { engine::fallback_title(conversation) } else { conversation.title.clone() };
    let mut text = format!("# {title}\n\n");
    for turn in store::turns(conversation) {
        text.push_str(if turn.role == "user" { "## You\n\n" } else { "## Assistant\n\n" });
        for part in turn.parts {
            match part {
                store::Part::Text { text: t } => text.push_str(&format!("{t}\n\n")),
                store::Part::Tool { name, .. } => text.push_str(&format!("_Used {name}_\n\n")),
                store::Part::Image { .. } => text.push_str("_[image]_\n\n"),
                store::Part::Reasoning { .. } => {}
            }
        }
    }
    text
}

#[tauri::command]
pub fn chat_list_apps(app: AppHandle) -> Result<Vec<super::canvas::Canvas>, String> {
    Ok(super::canvas::list_apps(&config_dir(&app)?))
}

/// Manages chat state at startup.
pub fn setup(app: &tauri::App) {
    app.manage(ChatState::default());
    if let Ok(dir) = config_dir(app.handle()) {
        super::store::purge_temporary(&dir);
    }
    super::tasks::start(app.handle().clone());
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
