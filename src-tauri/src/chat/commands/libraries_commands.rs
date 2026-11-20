//! Libraries: documents, uploads, folders kept in sync, and semantic search.

use super::*;

/// The text of an attached PDF or Office document, with its line breaks.
#[tauri::command]
pub async fn chat_read_document(
    app: AppHandle,
    name: String,
    data: String,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err("That document is too large.".into());
    }
    let engine = crate::chat::extraction::load(&config_dir(&app)?);
    if engine.external() {
        match crate::chat::extraction::convert(&engine, &name, bytes.clone()).await {
            Ok(text) => return Ok(text),
            Err(e) => log::warn!("{name}: {e}; using the built-in reader"),
        }
    }
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    tokio::task::spawn_blocking(move || {
        let text = if ext == "pdf" {
            // The PDF reader takes a path.
            let tmp = std::env::temp_dir().join(format!(
                "cortex-attachment-{}.pdf",
                uuid::Uuid::new_v4().simple()
            ));
            crate::chat::at_rest::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let text = crate::chat::libraries::read_text(&tmp);
            let _ = std::fs::remove_file(&tmp);
            text?
        } else if crate::chat::libraries::OFFICE_EXTENSIONS.contains(&ext.as_str()) {
            crate::chat::libraries::office_text(&bytes, &ext)?
        } else if ext == "zip" {
            crate::chat::libraries::zip_text(&bytes, 400_000)?
        } else {
            return Err(
                "Only PDF, Word, PowerPoint and OpenDocument files are read this way.".to_string(),
            );
        };
        if text.trim().is_empty() {
            return Err(if ext == "pdf" {
                "This PDF has no text layer (a scan?). Attach its pages as images instead."
                    .to_string()
            } else {
                "This document has no text.".to_string()
            });
        }
        Ok(text.trim().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn chat_list_libraries(app: AppHandle) -> Result<Vec<crate::chat::libraries::Library>, String> {
    Ok(crate::chat::libraries::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_create_library(
    app: AppHandle,
    name: String,
    description: String,
) -> Result<crate::chat::libraries::Library, String> {
    crate::chat::libraries::create(&config_dir(&app)?, &name, &description).map_err(String::from)
}

#[tauri::command]
pub fn chat_delete_library(app: AppHandle, id: String) -> Result<(), String> {
    crate::chat::libraries::delete(&config_dir(&app)?, &id).map_err(String::from)
}

/// Asks for files and adds each to the library, reporting the ones that could not be read.
#[tauri::command]
pub async fn chat_add_documents(app: AppHandle, id: String) -> Result<Option<AddResult>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(paths) = crate::support::dialogs::pick_files(
        app.dialog().file().set_title("Add documents").add_filter(
            "Documents",
            &[
                "pdf", "docx", "pptx", "odt", "odp", "md", "txt", "html", "htm", "csv", "json",
                "yaml", "yml", "xml", "rs", "py", "js", "ts", "go", "java", "c", "cpp", "h", "sql",
                "log", "png", "jpg", "jpeg", "webp", "gif",
            ],
        ),
    )
    .await?
    else {
        return Ok(None);
    };
    let dir = config_dir(&app)?;
    let mut errors: Vec<String> = Vec::new();
    for path in paths {
        let Ok(path) = path.into_path() else { continue };
        // Photos and scans of documents: the default model reads their text.
        if let Some(media_type) = crate::chat::libraries::is_image(&path) {
            let name = path
                .file_name()
                .map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned());
            match ocr(&dir, &path, media_type).await {
                Ok(text) => {
                    if let Err(e) = crate::chat::libraries::add_text(&dir, &id, &name, &text) {
                        errors.push(e.to_string());
                    }
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
            continue;
        }
        // A Tika or Docling server reads the file when one is set; the built-in readers otherwise.
        let engine = crate::chat::extraction::load(&dir);
        if engine.external() {
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            if let Ok(bytes) = std::fs::read(&path) {
                match crate::chat::extraction::convert(&engine, &name, bytes).await {
                    Ok(text) => {
                        if let Err(e) = crate::chat::libraries::add_text(&dir, &id, &name, &text) {
                            errors.push(e.to_string());
                        }
                        continue;
                    }
                    Err(e) => log::warn!("{name}: {e}; using the built-in reader"),
                }
            }
        }
        if let Err(e) = tokio::task::spawn_blocking({
            let dir = dir.clone();
            let id = id.clone();
            move || crate::chat::libraries::add(&dir, &id, &path)
        })
        .await
        .map_err(|e| e.to_string())?
        {
            errors.push(e.to_string());
        }
    }
    Ok(Some(AddResult {
        library: crate::chat::libraries::load(&dir, &id)?,
        errors,
    }))
}

#[tauri::command]
pub async fn chat_remove_library_folder(
    app: AppHandle,
    id: String,
    folder: String,
) -> Result<crate::chat::libraries::SyncReport, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::libraries::remove_folder(&dir, &id, &folder))
        .await
        .map_err(|e| e.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn chat_sync_library(
    app: AppHandle,
    id: String,
) -> Result<crate::chat::libraries::SyncReport, String> {
    let dir = config_dir(&app)?;
    let (d, i) = (dir.clone(), id.clone());
    let mut report = tokio::task::spawn_blocking(move || crate::chat::libraries::sync(&d, &i))
        .await
        .map_err(|e| e.to_string())??;
    if !crate::chat::libraries::load(&dir, &id)?
        .connectors
        .is_empty()
    {
        let more = crate::chat::libraries::sync_connectors(
            &dir,
            &id,
            &crate::chat::tools::registry_connectors(&app),
        )
        .await?;
        report.added += more.added;
        report.updated += more.updated;
        report.removed += more.removed;
        report.errors.extend(more.errors);
    }
    Ok(report)
}

/// The connectors a library can mirror documents from.
#[tauri::command]
pub fn chat_library_connectors(
    app: AppHandle,
    id: String,
) -> Result<Vec<LibraryConnector>, String> {
    use crate::tools::registry::Connector as _;
    let library = crate::chat::libraries::load(&config_dir(&app)?, &id)?;
    let mut list: Vec<LibraryConnector> = app
        .state::<crate::tools::registry::ToolRegistry>()
        .connectors()
        .into_iter()
        .map(|c| LibraryConnector {
            mirrored: library.connectors.contains(&c.id().to_string()),
            id: c.id().to_string(),
            name: c.descriptor().display_name,
        })
        .collect();
    list.sort_by_key(|a| a.name.to_lowercase());
    Ok(list)
}

/// Starts or stops mirroring a connector's documents into a library, then syncs it.
#[tauri::command]
pub async fn chat_set_library_connector(
    app: AppHandle,
    id: String,
    connector_id: String,
    on: bool,
) -> Result<crate::chat::libraries::SyncReport, String> {
    let dir = config_dir(&app)?;
    crate::chat::libraries::set_connector(&dir, &id, &connector_id, on)?;
    let report = crate::chat::libraries::sync_connectors(
        &dir,
        &id,
        &crate::chat::tools::registry_connectors(&app),
    )
    .await?;
    if on
        && report.added == 0
        && report.errors.is_empty()
        && !crate::chat::libraries::load(&dir, &id)?
            .documents
            .iter()
            .any(|d| {
                d.source
                    .as_deref()
                    .is_some_and(|s| s.starts_with(&format!("mcp:{connector_id}:")))
            })
    {
        return Err(
            "This connector offers no documents to index (it has tools but no resources).".into(),
        );
    }
    Ok(report)
}

#[tauri::command]
pub fn chat_remove_document(
    app: AppHandle,
    library_id: String,
    document_id: String,
) -> Result<crate::chat::libraries::Library, String> {
    crate::chat::libraries::remove(&config_dir(&app)?, &library_id, &document_id)
        .map_err(String::from)
}

/// Moves a document to another library.
#[tauri::command]
pub fn chat_move_document(
    app: AppHandle,
    library_id: String,
    document_id: String,
    to: String,
) -> Result<(), String> {
    crate::chat::libraries::move_document(&config_dir(&app)?, &library_id, &document_id, &to)
        .map_err(String::from)
}

/// Saves a web page's text into a library.
#[tauri::command]
pub async fn chat_add_web_page(
    app: AppHandle,
    library_id: String,
    url: String,
) -> Result<crate::chat::libraries::Library, String> {
    crate::chat::libraries::add_web_page(&config_dir(&app)?, &library_id, &url)
        .await
        .map_err(String::from)
}

/// The image names of an installed emoji pack (code points joined by hyphens).
#[tauri::command]
pub fn chat_emoji_index(app: AppHandle, style: String) -> Result<Vec<String>, String> {
    Ok(crate::chat::emoji_pack::index(&emoji_root(&app)?, &style))
}

/// Every file and image sent in a chat, newest first.
#[tauri::command]
pub async fn chat_uploads(app: AppHandle) -> Result<Vec<crate::chat::uploads::Upload>, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::uploads::list(&dir))
        .await
        .map_err(|e| e.to_string())
}

/// Takes a file or an image out of a chat's message.
#[tauri::command]
pub fn chat_remove_upload(
    app: AppHandle,
    conversation_id: String,
    message_index: usize,
    kind: String,
    name: String,
) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut c = store::load_conversation(&dir, &conversation_id)?;
    crate::chat::uploads::remove(&mut c, message_index, &kind, &name)?;
    store::save_conversation(&dir, &c).map_err(String::from)
}

/// Puts a large attachment in the "Uploads" library (made when missing), for the assistant to
/// search instead of reading it all; returns the library's name.
#[tauri::command]
pub fn chat_upload_to_library(
    app: AppHandle,
    name: String,
    text: String,
) -> Result<String, String> {
    let dir = config_dir(&app)?;
    let library = match crate::chat::libraries::list(&dir)
        .into_iter()
        .find(|l| l.name == "Uploads")
    {
        Some(l) => l,
        None => crate::chat::libraries::create(
            &dir,
            "Uploads",
            "Large files attached in chats, searched instead of sent in full.",
        )?,
    };
    crate::chat::libraries::add_text(&dir, &library.id, &name, &text)?;
    Ok(library.name)
}

/// The indexed text of a library document, for its preview.
#[tauri::command]
pub fn chat_document_text(
    app: AppHandle,
    library_id: String,
    document_id: String,
) -> Result<String, String> {
    crate::chat::libraries::document_text(&config_dir(&app)?, &library_id, &document_id, 200_000)
        .map_err(String::from)
}

#[tauri::command]
pub fn chat_embedding_settings(
    app: AppHandle,
) -> Result<crate::chat::embeddings::EmbeddingSettings, String> {
    Ok(crate::chat::embeddings::load(&config_dir(&app)?))
}

/// Saves semantic-search settings; a key given is stored in the keychain.
#[tauri::command]
pub async fn chat_save_embedding_settings(
    app: AppHandle,
    settings: crate::chat::embeddings::EmbeddingSettings,
    key: Option<String>,
    clear_key: bool,
    rerank_key: Option<String>,
    qdrant_key: Option<String>,
) -> Result<crate::chat::embeddings::EmbeddingSettings, String> {
    let dir = config_dir(&app)?;
    let mut settings = settings;
    let saved = crate::chat::embeddings::load(&dir);
    settings.has_key = saved.has_key;
    settings.has_rerank_key = saved.has_rerank_key;
    settings.has_qdrant_key = saved.has_qdrant_key;
    if let Some(key) = qdrant_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        secrets()?
            .store(crate::chat::embeddings::QDRANT_SECRET, &key)
            .await?;
        settings.has_qdrant_key = true;
    }
    if let Some(key) = rerank_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        secrets()?
            .store(crate::chat::embeddings::RERANK_SECRET, &key)
            .await?;
        settings.has_rerank_key = true;
    }
    if clear_key {
        secrets()?.delete(crate::chat::embeddings::SECRET).await?;
        settings.has_key = false;
    }
    if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        secrets()?
            .store(crate::chat::embeddings::SECRET, &key)
            .await?;
        settings.has_key = true;
    }
    crate::chat::embeddings::save(&dir, &settings)?;
    Ok(crate::chat::embeddings::load(&dir))
}

/// Embeds every library passage not embedded yet, so the first search is quick. Returns how many.
#[tauri::command]
pub async fn chat_index_libraries(app: AppHandle) -> Result<usize, String> {
    let dir = config_dir(&app)?;
    let settings = crate::chat::embeddings::load(&dir);
    if !crate::chat::embeddings::ready(&settings) {
        return Err("Turn on semantic search and choose its model first.".into());
    }
    crate::chat::embeddings::index(&dir, &settings, &[])
        .await
        .map_err(String::from)
}

/// The project's file library, created on first use.
#[tauri::command]
pub fn chat_project_library(
    app: AppHandle,
    project_id: String,
) -> Result<crate::chat::libraries::Library, String> {
    let dir = config_dir(&app)?;
    let mut projects = store::load_projects(&dir);
    let project = projects
        .iter_mut()
        .find(|p| p.id == project_id)
        .ok_or("No such project.")?;
    if let Some(id) = &project.library_id {
        if let Ok(library) = crate::chat::libraries::load(&dir, id) {
            return Ok(library);
        }
    }
    let library = crate::chat::libraries::create(
        &dir,
        &format!("{} files", project.name),
        "Files of the project",
    )?;
    project.library_id = Some(library.id.clone());
    store::save_projects(&dir, &projects)?;
    Ok(library)
}
