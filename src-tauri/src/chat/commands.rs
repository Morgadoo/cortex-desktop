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
    let dir = config_dir(&app)?;
    let settings = super::audio::load(&dir).stt;
    super::audio::transcribe(&dir, bytes, &format!("speech.{ext}"), &settings).await
}

/// Reads text aloud; returns base64 audio and its media type.
#[tauri::command]
pub async fn chat_speak(app: AppHandle, text: String) -> Result<SpeechOut, String> {
    use base64::Engine as _;
    let dir = config_dir(&app)?;
    let settings = super::audio::load(&dir).tts;
    let prose = super::media::speakable(&text);
    if prose.is_empty() {
        return Err("There is nothing to read.".into());
    }
    Ok(match super::audio::speak(&dir, &prose, &settings).await? {
        Some((audio, mime)) => SpeechOut::Audio { data: base64::engine::general_purpose::STANDARD.encode(audio), mime },
        // The page speaks it with the system voice.
        None => SpeechOut::Browser { text: prose, rate: settings.speed, voice: settings.voice },
    })
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpeechOut {
    Audio { data: String, mime: String },
    Browser { text: String, rate: f32, voice: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioView {
    pub settings: super::audio::AudioSettings,
    /// What the `provider` engines resolve to now.
    pub provider_speech_model: Option<String>,
    pub provider_voice_model: Option<String>,
}

#[tauri::command]
pub fn chat_audio_settings(app: AppHandle) -> Result<AudioView, String> {
    let dir = config_dir(&app)?;
    let file = super::providers::load(&dir);
    Ok(AudioView {
        settings: super::audio::load(&dir),
        provider_speech_model: super::media::speech_model(&file),
        provider_voice_model: super::media::voice_model(&file),
    })
}

/// Saves the audio settings; a key is stored in the keychain when given, removed when cleared.
#[tauri::command]
pub async fn chat_save_audio_settings(
    app: AppHandle,
    settings: super::audio::AudioSettings,
    stt_key: Option<String>,
    tts_key: Option<String>,
    clear_stt_key: bool,
    clear_tts_key: bool,
) -> Result<AudioView, String> {
    let dir = config_dir(&app)?;
    let previous = super::audio::load(&dir);
    let mut settings = super::audio::normalize(settings);
    settings.stt.has_key = previous.stt.has_key;
    settings.tts.has_key = previous.tts.has_key;
    for (key, clear, name, has) in [
        (stt_key, clear_stt_key, super::audio::STT_SECRET, &mut settings.stt.has_key),
        (tts_key, clear_tts_key, super::audio::TTS_SECRET, &mut settings.tts.has_key),
    ] {
        if clear {
            secrets()?.delete(name).await?;
            *has = false;
        }
        if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            secrets()?.store(name, &key).await?;
            *has = true;
        }
    }
    super::audio::save(&dir, &settings)?;
    chat_audio_settings(app)
}

/// Voices for a text-to-speech engine: the account's voices for ElevenLabs, the known ones else.
#[tauri::command]
pub async fn chat_audio_voices(engine: String, base_url: String) -> Result<Vec<(String, String)>, String> {
    if engine == "elevenlabs" {
        return super::audio::elevenlabs_voices(base_url.trim().trim_end_matches('/')).await;
    }
    Ok(super::audio::known_voices(&engine))
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

#[tauri::command]
pub fn chat_terminal_list(conversation_id: String) -> Vec<super::terminal::CommandView> {
    super::terminal::list(&conversation_id)
}

/// Starts a background command the user typed in the Terminal panel.
#[tauri::command]
pub async fn chat_terminal_start(app: AppHandle, conversation_id: String, command: String) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    super::terminal::start(&conversation_id, &dir, &command)
}

#[tauri::command]
pub fn chat_terminal_stop(id: String) -> Result<(), String> {
    super::terminal::stop(&id)
}

#[tauri::command]
pub fn chat_terminal_clear(conversation_id: String) {
    super::terminal::clear_finished(&conversation_id);
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

/// Saves a conversation's own model settings.
#[tauri::command]
pub fn chat_set_params(app: AppHandle, id: String, params: store::ChatParams) -> Result<store::ChatParams, String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    conversation.params = params.clamped();
    store::save_conversation(&dir, &conversation)?;
    Ok(conversation.params)
}

/// Three short messages the user might send next, from the latest exchange (Open WebUI's
/// follow-up prompts). Empty when the provider gives nothing usable.
#[tauri::command]
pub async fn chat_follow_ups(app: AppHandle, id: String, model: String) -> Result<Vec<String>, String> {
    let dir = config_dir(&app)?;
    let conversation = store::load_conversation(&dir, &id)?;
    let turns = store::turns(&conversation);
    let text_of = |t: &store::Turn| {
        t.parts.iter().filter_map(|p| if let store::Part::Text { text } = p { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join("\n")
    };
    let Some(answer) = turns.iter().rev().find(|t| t.role == "assistant").map(text_of).filter(|t| !t.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let question = turns.iter().rev().find(|t| t.role == "user").map(text_of).unwrap_or_default();
    let model = engine::connect_model(&dir, &model).await?;
    let request = rig_core::completion::CompletionRequest::new(rig_core::completion::message::Message::user(follow_up_prompt(&question, &answer)))
        .max_tokens(200u64);
    let response = tokio::time::timeout(std::time::Duration::from_secs(30), model.call(request))
        .await
        .map_err(|_| "The provider did not answer in time.".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(parse_follow_ups(&response.text()))
}

fn follow_up_prompt(question: &str, answer: &str) -> String {
    let cut = |t: &str, n: usize| t.chars().take(n).collect::<String>();
    format!(
        "Suggest three short follow-up messages the user might send next in this conversation, \
         written as the user, in the user's language. Each under 12 words, each a different \
         direction (go deeper, apply it, an alternative). One per line, no numbering, no quotes.\n\n\
         User: {}\n\nAssistant: {}",
        cut(question, 2_000),
        cut(answer, 4_000)
    )
}

/// Lines of the reply, without list marks or quotes, at most three.
fn parse_follow_ups(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | '*' | '•' | '.' | ')')).trim())
        .map(|l| l.trim_matches(|c| matches!(c, '"' | '“' | '”')).trim().to_string())
        .filter(|l| !l.is_empty() && l.chars().count() <= 120 && !l.ends_with(':'))
        .take(3)
        .collect()
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
    let web_app = canvas.kind == "app";
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file();
    let dialog = if web_app {
        // A single HTML file that runs in any browser.
        dialog.set_file_name(format!("{}.html", sanitize_file_name(&canvas.title))).add_filter("Web page", &["html"])
    } else if sheet {
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
        let sheets = super::sheets::split_sheets(canvas.content(), &canvas.title);
        let csv = sheets.first().map(|(_, c)| c.clone()).unwrap_or_default();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
            std::fs::write(&path, csv).map_err(|e| e.to_string())?;
        } else {
            // Every sheet into one workbook; CSV holds the first only.
            let rows: Vec<(String, Vec<Vec<String>>)> = sheets.iter().map(|(n, c)| (n.clone(), super::sheets::parse_csv(c))).collect();
            super::sheets::write_workbook(&path.with_extension("xlsx"), &rows)?;
        }
        return Ok(true);
    }
    if web_app {
        std::fs::write(path, super::canvas::app_html(canvas.content())).map_err(|e| e.to_string())?;
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
        .add_filter("Documents", &["pdf", "md", "txt", "html", "htm", "csv", "json", "yaml", "yml", "xml", "rs", "py", "js", "ts", "go", "java", "c", "cpp", "h", "sql", "log", "png", "jpg", "jpeg", "webp", "gif"])
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
        // Photos and scans of documents: the default model reads their text.
        if let Some(media_type) = super::libraries::is_image(&path) {
            let name = path.file_name().map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned());
            match ocr(&dir, &path, media_type).await {
                Ok(text) => {
                    if let Err(e) = super::libraries::add_text(&dir, &id, &name, &text) {
                        errors.push(e);
                    }
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
            continue;
        }
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

/// The text in an image, read by the default model: OCR with any model that can see.
async fn ocr(config_dir: &std::path::Path, path: &std::path::Path, media_type: &str) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let file = providers::load(config_dir);
    let reference = match file.default_model.clone() {
        Some(r) => r,
        None => providers::models(&file).await.first().map(|m| m.reference.clone()).ok_or("Set up a model first.")?,
    };
    let image = engine::ImageInput { media_type: media_type.into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) };
    let message = engine::user_message(OCR_PROMPT, &[image])?;
    let model = engine::connect_model(config_dir, &reference).await?;
    let request = rig_core::completion::CompletionRequest::new(message).max_tokens(8_000u64);
    let response = tokio::time::timeout(std::time::Duration::from_secs(180), model.call(request))
        .await
        .map_err(|_| "The model did not answer in time.".to_string())?
        .map_err(|e| format!("{} could not read it ({}); it needs a model that can see images.", reference, super::context::short_error(&e.to_string())))?;
    let text = response.text();
    if text.trim().eq_ignore_ascii_case("NO TEXT") {
        return Err("no text was found in the image.".into());
    }
    Ok(text)
}

const OCR_PROMPT: &str = "Transcribe all the text in this image exactly, in reading order, keeping headings, lists and \
    table rows (as Markdown). Describe charts, photos and diagrams in one short line each, in square brackets. \
    Output only the transcription. If there is no text at all, reply NO TEXT.";

#[tauri::command]
pub fn chat_remove_document(app: AppHandle, library_id: String, document_id: String) -> Result<super::libraries::Library, String> {
    super::libraries::remove(&config_dir(&app)?, &library_id, &document_id)
}

#[tauri::command]
pub async fn chat_search_libraries(app: AppHandle, query: String, library_id: Option<String>) -> Result<Vec<super::libraries::Passage>, String> {
    let only: Vec<String> = library_id.into_iter().collect();
    Ok(super::embeddings::search(&config_dir(&app)?, &only, &query, 8).await.0)
}

/// Removes a message the user sent and the reply to it.
#[tauri::command]
pub fn chat_delete_exchange(app: AppHandle, state: State<'_, ChatState>, id: String, index: usize) -> Result<(), String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish first.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::delete_exchange(&mut conversation, index)?;
    store::save_conversation(&dir, &conversation)
}

/// Saves an agent, its notebook and the skills it uses as one file, to back up or share.
#[tauri::command]
pub async fn chat_export_agent(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let dir = config_dir(&app)?;
    let bundle = super::agents::bundle(&dir, &id)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{}.lumo-agent.json", sanitize_file_name(&bundle.agent.name)))
        .add_filter("Agent", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Adds an agent from an exported file; returns its name, or `None` when cancelled.
#[tauri::command]
pub async fn chat_import_agent(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import an agent")
        .add_filter("Agent", &["json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let bundle: super::agents::Bundle = serde_json::from_str(&text).map_err(|_| "This file is not an exported agent.".to_string())?;
    let agent = super::agents::install(&config_dir(&app)?, bundle)?;
    Ok(Some(agent.name))
}

/// Writes a conversation as JSON, every message and tool step included, to a file the user picks.
#[tauri::command]
pub async fn chat_export_conversation_json(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    let view = store::view(&conversation);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{}.json", sanitize_file_name(&conversation.title)))
        .add_filter("JSON", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let out = serde_json::json!({
        "title": view.title,
        "createdAt": conversation.created_at,
        "updatedAt": conversation.updated_at,
        "model": conversation.model,
        "tags": conversation.tags,
        "usage": conversation.usage,
        "turns": view.turns,
    });
    std::fs::write(path, serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServerView {
    settings: super::api_server::ApiServerSettings,
    status: super::api_server::Status,
    /// Shown so the user can paste it into other tools; only once the server was turned on.
    token: Option<String>,
}

async fn api_server_view(app: &AppHandle) -> Result<ApiServerView, String> {
    let settings = super::api_server::load(&config_dir(app)?);
    let status = app.state::<super::api_server::ApiServer>().status();
    let token = if settings.enabled { Some(super::api_server::token(false).await?) } else { None };
    Ok(ApiServerView { settings, status, token })
}

#[tauri::command]
pub async fn chat_api_server(app: AppHandle) -> Result<ApiServerView, String> {
    api_server_view(&app).await
}

/// Saves the local API's settings and starts, restarts or stops it to match.
#[tauri::command]
pub async fn chat_save_api_server(app: AppHandle, settings: super::api_server::ApiServerSettings, regenerate_token: bool) -> Result<ApiServerView, String> {
    super::api_server::save(&config_dir(&app)?, &settings)?;
    if regenerate_token {
        super::api_server::token(true).await?;
    }
    super::api_server::apply(&app).await;
    api_server_view(&app).await
}

/// Planned task runs between two times, for the Scheduled page's calendar.
#[tauri::command]
pub fn chat_task_calendar(app: AppHandle, from: chrono::DateTime<chrono::Utc>, to: chrono::DateTime<chrono::Utc>) -> Result<Vec<super::tasks::Occurrence>, String> {
    if to < from || to - from > chrono::Duration::days(62) {
        return Err("Ask for at most two months.".into());
    }
    Ok(super::tasks::occurrences(&super::tasks::load(&config_dir(&app)?), from, to))
}

#[tauri::command]
pub fn chat_openapi_servers(app: AppHandle) -> Result<Vec<super::openapi::Server>, String> {
    Ok(super::openapi::list(&config_dir(&app)?))
}

/// Adds an OpenAPI tool server from its spec address, with an optional bearer key.
#[tauri::command]
pub async fn chat_add_openapi_server(app: AppHandle, name: String, spec_url: String, key: Option<String>) -> Result<Vec<super::openapi::Server>, String> {
    let dir = config_dir(&app)?;
    let servers = super::openapi::add(&dir, &name, &spec_url).await?;
    match (key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()), servers.last()) {
        (Some(key), Some(added)) => {
            secrets()?.store(&super::openapi::secret_name(&added.id), &key).await?;
            super::openapi::update(&dir, &added.id.clone(), None, Some(true))
        }
        _ => Ok(servers),
    }
}

#[tauri::command]
pub async fn chat_refresh_openapi_server(app: AppHandle, id: String) -> Result<Vec<super::openapi::Server>, String> {
    super::openapi::refresh(&config_dir(&app)?, &id).await
}

#[tauri::command]
pub fn chat_toggle_openapi_server(app: AppHandle, id: String, enabled: bool) -> Result<Vec<super::openapi::Server>, String> {
    super::openapi::update(&config_dir(&app)?, &id, Some(enabled), None)
}

#[tauri::command]
pub async fn chat_remove_openapi_server(app: AppHandle, id: String) -> Result<Vec<super::openapi::Server>, String> {
    let _ = secrets()?.delete(&super::openapi::secret_name(&id)).await;
    super::openapi::remove(&config_dir(&app)?, &id)
}

#[tauri::command]
pub fn chat_computer_settings(app: AppHandle) -> Result<super::computer::ComputerSettings, String> {
    Ok(super::computer::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_computer_settings(app: AppHandle, settings: super::computer::ComputerSettings) -> Result<(), String> {
    super::computer::save(&config_dir(&app)?, &settings)
}

#[tauri::command]
pub fn chat_search_settings(app: AppHandle) -> Result<super::search::SearchSettings, String> {
    Ok(super::search::load_settings(&config_dir(&app)?))
}

/// Saves the web search engine; a key given is stored in the keychain.
#[tauri::command]
pub async fn chat_save_search_settings(app: AppHandle, settings: super::search::SearchSettings, key: Option<String>) -> Result<super::search::SearchSettings, String> {
    let dir = config_dir(&app)?;
    let mut settings = settings;
    settings.has_key = super::search::load_settings(&dir).has_key;
    if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        secrets()?.store(super::search::SECRET, &key).await?;
        settings.has_key = true;
    }
    super::search::save_settings(&dir, &settings)?;
    Ok(super::search::load_settings(&dir))
}

#[tauri::command]
pub fn chat_embedding_settings(app: AppHandle) -> Result<super::embeddings::EmbeddingSettings, String> {
    Ok(super::embeddings::load(&config_dir(&app)?))
}

/// Saves semantic-search settings; a key given is stored in the keychain.
#[tauri::command]
pub async fn chat_save_embedding_settings(
    app: AppHandle,
    settings: super::embeddings::EmbeddingSettings,
    key: Option<String>,
    clear_key: bool,
) -> Result<super::embeddings::EmbeddingSettings, String> {
    let dir = config_dir(&app)?;
    let mut settings = settings;
    settings.has_key = super::embeddings::load(&dir).has_key;
    if clear_key {
        secrets()?.delete(super::embeddings::SECRET).await?;
        settings.has_key = false;
    }
    if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        secrets()?.store(super::embeddings::SECRET, &key).await?;
        settings.has_key = true;
    }
    super::embeddings::save(&dir, &settings)?;
    Ok(super::embeddings::load(&dir))
}

/// Embeds every library passage not embedded yet, so the first search is quick. Returns how many.
#[tauri::command]
pub async fn chat_index_libraries(app: AppHandle) -> Result<usize, String> {
    let dir = config_dir(&app)?;
    let settings = super::embeddings::load(&dir);
    if !super::embeddings::ready(&settings) {
        return Err("Turn on semantic search and set its server and model first.".into());
    }
    super::embeddings::index(&dir, &settings, &[]).await
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
                store::Part::Notice { text: t } => text.push_str(&format!("> {t}\n\n")),
            }
        }
    }
    text
}

#[tauri::command]
pub fn chat_list_apps(app: AppHandle) -> Result<Vec<super::canvas::Canvas>, String> {
    Ok(super::canvas::list_apps(&config_dir(&app)?))
}


/// An agent's notebook.
#[tauri::command]
pub fn chat_agent_notebook(app: AppHandle, id: String) -> Result<String, String> {
    Ok(super::agents::notebook(&config_dir(&app)?, &id))
}

#[tauri::command]
pub fn chat_save_agent_notebook(app: AppHandle, id: String, text: String) -> Result<(), String> {
    super::agents::save_notebook(&config_dir(&app)?, &id, &text)
}

/// What each agent's conversations cost this month, by agent id.
#[tauri::command]
pub fn chat_agent_spend(app: AppHandle) -> Result<std::collections::HashMap<String, super::agents::Spend>, String> {
    Ok(super::agents::spend_this_month(&config_dir(&app)?))
}

/// Drafts an agent from a description, with the default model.
#[tauri::command]
pub async fn chat_draft_agent(app: AppHandle, description: String) -> Result<super::agents::Agent, String> {
    use rig_core::completion::CompletionRequest;
    let dir = config_dir(&app)?;
    if description.trim().len() < 8 {
        return Err("Describe the agent in a sentence or two.".into());
    }
    let reference = providers::load(&dir).default_model.ok_or("Choose a default model under Providers & models first.")?;
    let model = engine::connect_model(&dir, &reference).await?;
    let tools: Vec<&str> = TOOL_CHOICES.to_vec();
    let skills: Vec<String> = super::skills::list(&dir).into_iter().filter(|s| s.enabled).map(|s| s.name).collect();
    let request = CompletionRequest::new(rig_core::completion::Message::user(super::agents::draft_prompt(description.trim(), &tools, &skills)));
    let response = model.call(request).await.map_err(|e| e.to_string())?;
    super::agents::parse_draft(&response.text(), &tools, &skills)
}

/// The tool names an agent can be given, matching the editor's choices.
const TOOL_CHOICES: &[&str] = &["web_search", "browser", "filesystem", "bash", "delegate", "update_plan", "canvas", "schedule_task", "use_skill", "remember"];

/// Tokens and cost across all conversations this calendar month, for the usage gauge.
#[tauri::command]
pub fn chat_usage_month(app: AppHandle) -> Result<super::agents::Spend, String> {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let mut total = super::agents::Spend::default();
    for c in store::list_conversations(&config_dir(&app)?) {
        if c.updated_at.year() == now.year() && c.updated_at.month() == now.month() {
            total.runs += 1;
            total.tokens += c.tokens;
            total.cost += c.cost;
        }
    }
    Ok(total)
}

/// What a sub-agent did, step by step.
#[tauri::command]
pub fn chat_subagent_transcript(app: AppHandle, call_id: String, index: usize) -> Result<Vec<super::subagents::Step>, String> {
    Ok(super::subagents::transcript(&config_dir(&app)?, &call_id, index))
}

/// The files of a coding session's folder.
#[tauri::command]
pub async fn chat_code_files(app: AppHandle, conversation_id: String) -> Result<(Vec<String>, bool), String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || super::code::files(&dir)).await.map_err(|e| e.to_string())
}

/// One file of a coding session's folder, as text.
#[tauri::command]
pub fn chat_code_read_file(app: AppHandle, conversation_id: String, path: String) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation.workdir.ok_or("This conversation has no folder.")?;
    super::code::read_file(&dir, &path)
}

/// A site's icon as a `data:` URL, for sources and citations; `None` when it has none.
#[tauri::command]
pub async fn chat_favicon(app: AppHandle, host: String) -> Result<Option<String>, String> {
    Ok(super::favicon::get(&config_dir(&app)?, &host).await)
}

/// A web page's title, description and preview image, for the hover card on a citation.
#[tauri::command]
pub async fn chat_link_preview(app: AppHandle, url: String) -> Result<super::preview::Preview, String> {
    super::preview::get(&config_dir(&app)?, &url).await
}

/// What a session holds, for the Debug panel: model, folder, usage, summary and the raw history.
#[tauri::command]
pub fn chat_debug(app: AppHandle, id: String) -> Result<serde_json::Value, String> {
    let c = store::load_conversation(&config_dir(&app)?, &id)?;
    Ok(serde_json::json!({
        "id": c.id,
        "model": c.model,
        "workdir": c.workdir,
        "agentId": c.agent_id,
        "createdAt": c.created_at,
        "updatedAt": c.updated_at,
        "usage": c.usage,
        "summary": c.summary,
        "messages": c.messages,
    }))
}

/// Saves a conversation the page rendered as one self-contained web page, to share.
#[tauri::command]
pub async fn chat_save_shared_page(app: AppHandle, title: String, html: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    if html.len() > 64 * 1024 * 1024 {
        return Err("The conversation is too large to save as one page.".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("{}.html", sanitize_file_name(&title)))
        .add_filter("Web page", &["html"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(path.with_extension("html"), html).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Full-text search over every conversation's title and messages.
#[tauri::command]
pub async fn chat_search(app: AppHandle, query: String) -> Result<Vec<super::index::Hit>, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || super::index::search(&dir, &query, 40)).await.map_err(|e| e.to_string())
}

/// Deleted conversations, kept for 30 days.
#[tauri::command]
pub fn chat_trash(app: AppHandle) -> Result<Vec<super::backup::TrashItem>, String> {
    Ok(super::backup::list_trash(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_restore_conversation(app: AppHandle, id: String) -> Result<(), String> {
    super::backup::restore(&config_dir(&app)?, &id)
}

/// Deletes one conversation from the trash for good, or empties it.
#[tauri::command]
pub fn chat_delete_forever(app: AppHandle, id: Option<String>) -> Result<(), String> {
    super::backup::delete_forever(&config_dir(&app)?, id.as_deref())
}

/// How the stored data is doing: counts, size, backups, and files that cannot be read.
#[tauri::command]
pub async fn chat_storage_health(app: AppHandle) -> Result<super::backup::Health, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || super::backup::health(&dir)).await.map_err(|e| e.to_string())
}

/// Replaces a damaged conversation with its newest backup; returns that backup's date.
#[tauri::command]
pub fn chat_recover_conversation(app: AppHandle, id: String) -> Result<String, String> {
    super::backup::recover(&config_dir(&app)?, &id)
}

/// What fills the model's context window in a conversation.
#[tauri::command]
pub fn chat_context_breakdown(app: AppHandle, id: String) -> Result<super::context::Breakdown, String> {
    let dir = config_dir(&app)?;
    let c = store::load_conversation(&dir, &id)?;
    let reference = c.model.clone().or_else(|| providers::load(&dir).default_model).unwrap_or_default();
    Ok(super::context::breakdown(&c.messages, c.summary.as_ref(), c.usage.context_tokens, super::context::context_window(&reference)))
}

/// Branches a conversation into a new one that continues from the same point.
#[tauri::command]
pub fn chat_fork_conversation(app: AppHandle, id: String) -> Result<String, String> {
    let dir = config_dir(&app)?;
    let mut c = store::load_conversation(&dir, &id)?;
    let title = if c.title.is_empty() { engine::fallback_title(&c) } else { c.title.clone() };
    c.id = uuid::Uuid::new_v4().to_string();
    c.title = format!("{title} (branch)");
    c.pinned = false;
    c.unread = false;
    c.temporary = false;
    c.archived = false;
    c.task_id = None;
    c.created_at = chrono::Utc::now();
    c.updated_at = c.created_at;
    store::save_conversation(&dir, &c)?;
    Ok(c.id)
}

/// Manages chat state at startup.
pub fn setup(app: &tauri::App) {
    app.manage(ChatState::default());
    if let Ok(dir) = config_dir(app.handle()) {
        super::store::purge_temporary(&dir);
        // Off the startup path: copying state files and pruning old backups can touch many files.
        std::thread::spawn(move || super::backup::daily(&dir));
    }
    super::tasks::start(app.handle().clone());
    app.manage(super::api_server::ApiServer::default());
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        super::api_server::apply(&handle).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_ups_are_clean_lines() {
        let got = parse_follow_ups("1. How do I start?\n- \"Show an example\"\n\nHere are some:\nWhat about costs?\nA fourth one");
        assert_eq!(got, vec!["How do I start?", "Show an example", "What about costs?"]);
        assert!(follow_up_prompt("q", "a").contains("three short follow-up"));
    }

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
