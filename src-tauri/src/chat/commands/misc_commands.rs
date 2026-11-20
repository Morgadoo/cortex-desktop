//! The remaining chat commands.

use super::*;

#[tauri::command]
pub fn chat_workflows(app: AppHandle) -> Result<Vec<crate::chat::workflows::WorkflowView>, String> {
    let dir = config_dir(&app)?;
    Ok(crate::chat::workflows::views(
        &dir,
        crate::chat::workflows::load(&dir),
    ))
}

#[tauri::command]
pub fn chat_save_workflow(
    app: AppHandle,
    workflow: crate::chat::workflows::Workflow,
) -> Result<Vec<crate::chat::workflows::WorkflowView>, String> {
    let dir = config_dir(&app)?;
    let all = crate::chat::workflows::save(&dir, workflow)?;
    Ok(crate::chat::workflows::views(&dir, all))
}

#[tauri::command]
pub fn chat_delete_workflow(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::workflows::WorkflowView>, String> {
    let dir = config_dir(&app)?;
    let all = crate::chat::workflows::delete(&dir, &id)?;
    let _ = crate::chat::versions::set_production(&dir, "workflows", &id, None);
    Ok(crate::chat::workflows::views(&dir, all))
}

/// Marks a version of an agent, workflow or tool as production (`value`), or clears the mark.
#[tauri::command]
pub fn chat_set_production(
    app: AppHandle,
    kind: String,
    id: String,
    value: Option<serde_json::Value>,
) -> Result<Option<crate::chat::versions::Production>, String> {
    crate::chat::versions::set_production(&config_dir(&app)?, &kind, &id, value)
}

/// Transcribes speech recorded in the composer (base64 audio). `live` marks a guess made while
/// the speaker is still talking (see `live.ts` in the page).
#[tauri::command]
pub async fn chat_transcribe(
    app: AppHandle,
    audio: String,
    mime: String,
    live: Option<bool>,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio.as_bytes())
        .map_err(|e| e.to_string())?;
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
    let settings = crate::chat::audio::load(&dir).stt;
    crate::chat::audio::transcribe(
        &dir,
        bytes,
        &format!("speech.{ext}"),
        &settings,
        live.unwrap_or(false),
    )
    .await
}

/// A desktop notification, e.g. when a reply finishes while the window is in the background.
#[tauri::command]
pub fn chat_notify(app: AppHandle, title: String, body: String) {
    let body: String = crate::support::text::clip(&body, 180);
    crate::chat::engine::notify(&app, title.trim(), body.trim());
}

/// Pushes a coding session's branch and opens a pull request for it.
/// The pull request of the chat folder's branch, with its review and CI checks.
/// A text file from a chat's workspace, to show as a side-panel tab.
#[tauri::command]
pub fn chat_workspace_text(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<String, String> {
    let root = crate::chat::sandbox::workspace(&config_dir(&app)?, &conversation_id)?;
    let real = root
        .join(&path)
        .canonicalize()
        .map_err(|_| format!("There is no file at {path}."))?;
    if !real.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Only files in this chat's workspace open here.".into());
    }
    let bytes = crate::chat::at_rest::read(&real).map_err(|e| e.to_string())?;
    if bytes.len() > 2_000_000 {
        return Err("The file is too large to show here; save it instead.".into());
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err("This file is not text; save it to open it in another app.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// What a running turn has streamed so far, for a window that opens during it.
#[tauri::command]
pub fn chat_live_events(
    state: State<'_, ChatState>,
    conversation_id: String,
) -> Vec<serde_json::Value> {
    state.replay(&conversation_id)
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
    crate::chat::audit::clear(&config_dir(&app)?)
}

/// Saves a conversation's own model settings.
#[tauri::command]
pub fn chat_set_params(
    app: AppHandle,
    id: String,
    params: store::ChatParams,
) -> Result<store::ChatParams, String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    conversation.params = params.clamped();
    store::save_conversation(&dir, &conversation)?;
    Ok(conversation.params)
}

/// Starts recording a task to teach: the main window steps aside for a small Recording pill.
#[tauri::command]
pub fn chat_record_start(
    app: AppHandle,
    recorder: tauri::State<'_, crate::chat::recorder::Recorder>,
) -> Result<(), String> {
    use tauri::Manager;
    recorder.start()?;
    let pill = match app.get_webview_window(crate::chat::recorder::PILL) {
        Some(w) => w,
        None => tauri::WebviewWindowBuilder::new(
            &app,
            crate::chat::recorder::PILL,
            tauri::WebviewUrl::App("record.html".into()),
        )
        .title("Recording")
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .inner_size(380.0, 84.0)
        .build()
        .map_err(|e| e.to_string())?,
    };
    if let Ok(Some(m)) = pill.primary_monitor() {
        let s = m.scale_factor();
        let x = m.position().x + (m.size().width as i32 - (380.0 * s) as i32) / 2;
        let _ = pill.set_position(tauri::PhysicalPosition::new(
            x,
            m.position().y + (24.0 * s) as i32,
        ));
    }
    let _ = pill.show();
    crate::main_window::hide(&app);
    Ok(())
}

#[tauri::command]
pub fn chat_record_status(
    recorder: tauri::State<'_, crate::chat::recorder::Recorder>,
) -> crate::chat::recorder::Status {
    recorder.status()
}

/// Stops the recording (from the pill), brings Cortex back and asks it to review the steps.
#[tauri::command]
pub async fn chat_record_stop(
    app: AppHandle,
    keep: bool,
) -> Result<crate::chat::recorder::Status, String> {
    use tauri::{Emitter, Manager};
    // Stopping waits for one last screenshot: off the main thread.
    let handle = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        let recorder = handle.state::<crate::chat::recorder::Recorder>();
        if keep {
            recorder.stop()
        } else {
            recorder.discard();
            recorder.status()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(pill) = app.get_webview_window(crate::chat::recorder::PILL) {
        let _ = pill.hide();
    }
    crate::main_window::show(&app)?;
    if keep {
        let _ = app.emit("chat://recorded", &status);
    }
    Ok(status)
}

#[tauri::command]
pub fn chat_record_discard(recorder: tauri::State<'_, crate::chat::recorder::Recorder>) {
    recorder.discard();
}

/// Brings the main window forward (the wake word heard while Cortex was in the background).
#[tauri::command]
pub fn chat_show_window(app: AppHandle) -> Result<(), String> {
    crate::main_window::show(&app)
}

/// Every translation of interface text known for `code`.
#[tauri::command]
pub fn chat_ui_translations(
    app: AppHandle,
    code: String,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    Ok(crate::chat::ui_language::translations(
        &config_dir(&app)?,
        &code,
    ))
}

/// How much of the interface `code` covers.
#[tauri::command]
pub fn chat_ui_coverage(
    app: AppHandle,
    code: String,
) -> Result<crate::chat::ui_language::Coverage, String> {
    Ok(crate::chat::ui_language::coverage(
        &config_dir(&app)?,
        &code,
    ))
}

/// A source read as an article, for the reader in the side panel.
#[tauri::command]
pub async fn chat_read_article(url: String) -> Result<crate::chat::reader::Article, String> {
    crate::chat::reader::read(&url).await
}

/// Removes a message the user sent and the reply to it.
#[tauri::command]
pub fn chat_delete_exchange(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
    index: usize,
) -> Result<(), String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish first.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::delete_exchange(&mut conversation, index)?;
    store::save_conversation(&dir, &conversation).map_err(String::from)
}

/// Opens a conversation in a window of its own (LobeHub's "open in new window"), or brings its
/// window forward when it is open already. The window runs the same page, told which chat to show.
#[tauri::command]
pub fn chat_open_chat_window(app: AppHandle, id: String) -> Result<(), String> {
    use tauri::Manager as _;
    if !store::valid_id(&id) {
        return Err("invalid conversation id".into());
    }
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    let label = format!(
        "cortex-chat-{}",
        id.chars()
            .filter(char::is_ascii_alphanumeric)
            .take(32)
            .collect::<String>()
    );
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    let handle = app.clone();
    let handle2 = app.clone();
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App(crate::webview::APP_PAGE.into()),
    )
    .title(if conversation.title.is_empty() {
        "Cortex".to_string()
    } else {
        conversation.title.clone()
    })
    .inner_size(900.0, 780.0)
    .min_inner_size(480.0, 420.0)
    .initialization_script(format!(
        "window.__CORTEX_SOLO_CHAT = {};",
        serde_json::to_string(&id).map_err(|e| e.to_string())?
    ))
    .disable_drag_drop_handler()
    .on_navigation(move |url| crate::webview::handle_navigation(&handle, url))
    .on_new_window(move |url, _features| crate::webview::handle_new_window(&handle2, &url))
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Saves bytes the page made (an image of a conversation) where the user picks.
#[tauri::command]
pub async fn chat_save_bytes(app: AppHandle, name: String, data: String) -> Result<bool, String> {
    use base64::Engine as _;
    use tauri_plugin_dialog::DialogExt;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    let ext = name.rsplit('.').next().unwrap_or("bin").to_string();
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(&name)
            .add_filter(ext.to_uppercase(), &[ext.as_str()]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(true)
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

/// The app link (cortex://…) waiting for the page, if any.
#[tauri::command]
pub fn chat_take_link(
    pending: State<'_, crate::chat::links::PendingLink>,
) -> Option<crate::chat::links::Link> {
    pending.take()
}

/// Deletes the exchanges (prompt and answer) holding the given messages; returns how many went.
#[tauri::command]
pub fn chat_delete_exchanges(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
    indices: Vec<usize>,
) -> Result<usize, String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish first.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    let count = store::delete_exchanges(&mut conversation, &indices)?;
    store::save_conversation(&dir, &conversation)?;
    Ok(count)
}

/// Runs a code block from an answer in the sandbox, at the user's click; returns its output.
#[tauri::command]
pub async fn chat_run_snippet(
    app: AppHandle,
    language: String,
    code: String,
) -> Result<String, String> {
    let outcome = crate::chat::sandbox::run(
        &config_dir(&app)?,
        "snippets",
        &serde_json::json!({ "language": language, "code": code }),
    )
    .await;
    if outcome.is_error {
        Err(outcome.text)
    } else {
        Ok(outcome.text)
    }
}

/// A continuation for the message being typed (Open WebUI's autocomplete): a few words, or
/// nothing when the message reads as finished.
#[tauri::command]
pub async fn chat_complete_prompt(
    app: AppHandle,
    model: String,
    text: String,
) -> Result<String, String> {
    let text: String = text
        .chars()
        .rev()
        .take(2_000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if text.trim().chars().count() < 8 {
        return Ok(String::new());
    }
    let prompt = format!(
        "The user is typing a message to an AI assistant. Suggest how their sentence most likely \
         continues: at most 12 words, in their language and voice, continuing exactly where the text \
         stops (start with a space when the next word is new). Reply with the continuation only, no \
         quotes. If the message already reads as complete, reply with nothing.\n\n<typed>{text}</typed>"
    );
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(600u64);
    let out = match engine::ask_text(&config_dir(&app)?, &model, request, 10).await {
        Ok(out) => out,
        Err(e) if e.contains("no text") => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let out = out
        .trim_end()
        .trim_start_matches(['"', '\''])
        .trim_end_matches(['"', '\''])
        .to_string();
    // A continuation, not a new message: keep it short and on one line.
    let out: String = out
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .take(14)
        .collect::<Vec<_>>()
        .join(" ");
    if out.is_empty() {
        return Ok(String::new());
    }
    let joined = if text.ends_with(' ') || out.starts_with([',', '.', '?', '!', ';', ':']) {
        out
    } else {
        format!(" {out}")
    };
    Ok(joined)
}

/// Checks the Qdrant server in the saved settings; returns its version.
#[tauri::command]
pub async fn chat_check_vector_store(app: AppHandle) -> Result<String, String> {
    let settings = crate::chat::embeddings::load(&config_dir(&app)?);
    if settings.qdrant_url.is_empty() {
        return Err("Enter the Qdrant server's address first.".into());
    }
    crate::chat::embeddings::qdrant::check(&settings).await
}

/// Sessions of other coding agents (Claude Code, Codex) on this computer, newest first.
#[tauri::command]
pub async fn chat_find_coding_sessions(
    app: AppHandle,
) -> Result<Vec<crate::chat::coding_import::Found>, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::coding_import::find(&dir, None))
        .await
        .map_err(|e| e.to_string())
}

/// Imports the chosen sessions as Code conversations; returns their ids.
#[tauri::command]
pub async fn chat_import_coding_sessions(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let dir = config_dir(&app)?;
    // Only files the search found can be imported.
    let found: Vec<String> = crate::chat::coding_import::find(&dir, None)
        .into_iter()
        .map(|f| f.path)
        .collect();
    tokio::task::spawn_blocking(move || {
        paths
            .iter()
            .filter(|p| found.contains(p))
            .map(|p| crate::chat::coding_import::import(&dir, std::path::Path::new(p)))
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A site's icon as a `data:` URL, for sources and citations; `None` when it has none.
#[tauri::command]
pub async fn chat_favicon(app: AppHandle, host: String) -> Result<Option<String>, String> {
    Ok(crate::chat::favicon::get(&config_dir(&app)?, &host).await)
}

/// Deletes one conversation from the trash for good, or empties it.
#[tauri::command]
pub fn chat_delete_forever(app: AppHandle, id: Option<String>) -> Result<(), String> {
    crate::chat::backup::delete_forever(&config_dir(&app)?, id.as_deref())
}

/// How the stored data is doing: counts, size, backups, and files that cannot be read.
#[tauri::command]
pub async fn chat_storage_health(app: AppHandle) -> Result<crate::chat::backup::Health, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::backup::health(&dir))
        .await
        .map_err(|e| e.to_string())
}
