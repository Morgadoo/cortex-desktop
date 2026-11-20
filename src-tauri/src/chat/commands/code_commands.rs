//! Code sessions: files, changes, the terminal, pull requests, snippets run from answers, and sessions imported from other coding agents.

use super::*;
use crate::chat::terminal::Terminals;

/// Whether code runs without asking each time.
#[tauri::command]
pub fn chat_code_runs_allowed(app: AppHandle) -> Result<bool, String> {
    Ok(crate::chat::sandbox::always_allowed(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_set_code_runs_allowed(app: AppHandle, allowed: bool) -> Result<(), String> {
    crate::chat::sandbox::set_always_allowed(&config_dir(&app)?, allowed).map_err(String::from)
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
pub fn chat_terminal_list(
    state: State<'_, ChatState>,
    conversation_id: String,
) -> Vec<crate::chat::terminal::CommandView> {
    state.part::<Terminals>().list(&conversation_id)
}

/// Starts a background command the user typed in the Terminal panel.
#[tauri::command]
pub async fn chat_terminal_start(
    app: AppHandle,
    state: State<'_, ChatState>,
    conversation_id: String,
    command: String,
) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &conversation_id)?;
    let dir = conversation
        .workdir
        .ok_or("This conversation has no folder.")?;
    state
        .part::<Terminals>()
        .start(&conversation_id, &dir, &command)
        .map_err(String::from)
}

#[tauri::command]
pub fn chat_terminal_stop(state: State<'_, ChatState>, id: String) -> Result<(), String> {
    state.part::<Terminals>().stop(&id).map_err(String::from)
}

#[tauri::command]
pub fn chat_terminal_clear(state: State<'_, ChatState>, conversation_id: String) {
    state.part::<Terminals>().clear_finished(&conversation_id);
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
        .map_err(String::from)
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
        .map_err(String::from)
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
    crate::chat::code::create_worktree(&dir, &name).map_err(String::from)
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
    crate::chat::recent_files::add(&config_dir(&app)?, files).map_err(String::from)
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
    crate::chat::code::read_file(&dir, &path).map_err(String::from)
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
    crate::chat::code::read_file(&dir, &path).map_err(String::from)
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

/// Runs a code block from an answer in the sandbox, at the user's click; returns its output.
#[tauri::command]
pub async fn chat_run_snippet(
    app: AppHandle,
    language: String,
    code: String,
) -> Result<String, String> {
    let outcome = crate::chat::sandbox::run(
        &app.state::<ChatState>()
            .part::<crate::chat::workbench::Bench>(),
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
    .map_err(String::from)
}
