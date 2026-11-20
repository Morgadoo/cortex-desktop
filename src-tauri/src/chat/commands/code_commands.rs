//! Code sessions: files, changes, the terminal and pull requests.

use super::*;

/// Whether code runs without asking each time.
#[tauri::command]
pub fn chat_code_runs_allowed(app: AppHandle) -> Result<bool, String> {
    Ok(crate::chat::sandbox::always_allowed(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_code_runs_allowed(app: AppHandle, allowed: bool) -> Result<(), String> {
    crate::chat::sandbox::set_always_allowed(&config_dir(&app)?, allowed)
}

/// The working tree's changes in a coding session's folder.
#[tauri::command]
pub async fn chat_code_changes(
    app: AppHandle,
    conversation_id: String,
) -> Result<crate::chat::code::Changes, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || crate::chat::code::changes(&dir))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn chat_terminal_list(conversation_id: String) -> Vec<crate::chat::terminal::CommandView> {
    crate::chat::terminal::list(&conversation_id)
}

/// Starts a background command the user typed in the Terminal panel.
#[tauri::command]
pub async fn chat_terminal_start(
    app: AppHandle,
    conversation_id: String,
    command: String,
) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    crate::chat::terminal::start(&conversation_id, &dir, &command)
}

#[tauri::command]
pub fn chat_terminal_stop(id: String) -> Result<(), String> {
    crate::chat::terminal::stop(&id)
}

#[tauri::command]
pub fn chat_terminal_clear(conversation_id: String) {
    crate::chat::terminal::clear_finished(&conversation_id);
}

/// Commits the working tree of a coding session's folder.
#[tauri::command]
pub async fn chat_code_commit(
    app: AppHandle,
    conversation_id: String,
    message: String,
) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || crate::chat::code::commit(&dir, &message))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn chat_code_pr_status(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<crate::chat::code::PrStatus>, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || crate::chat::code::pr_status(&dir))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_code_pull_request(
    app: AppHandle,
    conversation_id: String,
    title: String,
) -> Result<crate::chat::code::PullRequest, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    if title.trim().is_empty() {
        return Err("Give the pull request a title.".into());
    }
    tokio::task::spawn_blocking(move || crate::chat::code::open_pull_request(&dir, &title))
        .await
        .map_err(|e| e.to_string())?
}

/// Opens a workspace file (a video, a document) in the system's app for it. A file kept
/// encrypted is opened from a plain copy in the temporary folder.
#[tauri::command]
pub fn chat_open_workspace_file(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<(), String> {
    let real = workspace_file(&app, &conversation_id, &path)?;
    let bytes = crate::chat::at_rest::read(&real).map_err(|e| e.to_string())?;
    let on_disk = std::fs::read(&real).map_err(|e| e.to_string())?;
    let target = if bytes == on_disk {
        real
    } else {
        let dir = std::env::temp_dir().join("cortex-open");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let t = dir.join(real.file_name().ok_or("No such file.")?);
        std::fs::write(&t, bytes).map_err(|e| e.to_string())?;
        t
    };
    open::that_detached(&target).map_err(|e| format!("No app here opens this file: {e}"))
}

/// Makes a git worktree of a repository on a new branch for a Code session; returns its folder
/// and branch.
#[tauri::command]
pub fn chat_create_worktree(dir: String, name: String) -> Result<(String, String), String> {
    crate::chat::code::create_worktree(&dir, &name)
}

/// Saves bytes the page made (a code block, a diagram) to a file the user picks.
#[tauri::command]
pub async fn chat_save_file(app: AppHandle, name: String, data: String) -> Result<bool, String> {
    use base64::Engine as _;
    use tauri_plugin_dialog::DialogExt;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("The file is too large.".into());
    }
    let name: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':') && !c.is_control())
        .take(120)
        .collect();
    let Some(path) =
        crate::support::dialogs::save_file(app.dialog().file().set_file_name(if name.is_empty() {
            "file".to_string()
        } else {
            name
        }))
        .await?
    else {
        return Ok(false);
    };
    crate::chat::at_rest::write(path.into_path().map_err(|e| e.to_string())?, bytes)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Files attached lately, newest first, to attach again.
#[tauri::command]
pub fn chat_recent_files(
    app: AppHandle,
) -> Result<Vec<crate::chat::recent_files::RecentFile>, String> {
    Ok(crate::chat::recent_files::load(&config_dir(&app)?))
}

/// Remembers the files a message carried.
#[tauri::command]
pub fn chat_remember_files(
    app: AppHandle,
    files: Vec<crate::chat::recent_files::RecentFile>,
) -> Result<(), String> {
    crate::chat::recent_files::add(&config_dir(&app)?, files)
}

/// The files of a coding session's folder.
#[tauri::command]
pub async fn chat_code_files(
    app: AppHandle,
    conversation_id: String,
) -> Result<(Vec<String>, bool), String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    tokio::task::spawn_blocking(move || crate::chat::code::files(&dir))
        .await
        .map_err(|e| e.to_string())
}

/// A folder's files for `@` mentions: tracked and new ones, not ignored (git-aware).
#[tauri::command]
pub async fn chat_folder_files(dir: String) -> Result<(Vec<String>, bool), String> {
    if !std::path::Path::new(&dir).is_dir() {
        return Err("That folder does not exist.".into());
    }
    tokio::task::spawn_blocking(move || crate::chat::code::files(&dir))
        .await
        .map_err(|e| e.to_string())
}

/// One file of a folder, as text, for an `@` mention; paths that leave the folder are refused.
#[tauri::command]
pub fn chat_folder_read(dir: String, path: String) -> Result<String, String> {
    crate::chat::code::read_file(&dir, &path)
}

/// One file of a coding session's folder, as text.
#[tauri::command]
pub fn chat_code_read_file(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    crate::chat::code::read_file(&dir, &path)
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
