//! Exports and imports: conversations, canvases, agents, the activity log, diagnostics, everything, and archives and other apps' exports brought in.

use super::*;

/// Saves the activity log (the last `days`, or all) as CSV or JSON where the user chooses.
#[tauri::command]
pub async fn chat_export_audit(
    app: AppHandle,
    format: String,
    days: Option<i64>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let since = days
        .filter(|d| *d > 0)
        .map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    let entries = crate::chat::audit::entries(&config_dir(&app)?, since);
    let (text, ext) = if format == "json" {
        (
            serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?,
            "json",
        )
    } else {
        (crate::chat::audit::to_csv(&entries), "csv")
    };
    let Some(target) =
        crate::support::dialogs::save_file(app.dialog().file().set_file_name(format!(
            "cortex-activity-{}.{ext}",
            chrono::Local::now().format("%Y-%m-%d")
        )))
        .await?
    else {
        return Ok(false);
    };
    std::fs::write(target.into_path().map_err(|e| e.to_string())?, text)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Publishes skills as a catalogue folder the user chooses, to push to GitHub.
#[tauri::command]
pub async fn chat_export_skill_catalog(
    app: AppHandle,
    names: Vec<String>,
    title: String,
    author: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(folder) = crate::support::dialogs::pick_folder(
        app.dialog()
            .file()
            .set_title("Where to write the catalogue"),
    )
    .await?
    else {
        return Ok(None);
    };
    let folder = folder.into_path().map_err(|e| e.to_string())?;
    crate::chat::skill_catalog::export(&config_dir(&app)?, &names, &folder, &title, &author)?;
    Ok(Some(folder.to_string_lossy().into_owned()))
}

/// Imports a ChatGPT or Claude data export (the zip, or its conversations.json).
#[tauri::command]
pub async fn chat_import_conversations(
    app: AppHandle,
) -> Result<Option<crate::chat::import::ImportReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = crate::support::dialogs::pick_file(
        app.dialog()
            .file()
            .set_title("Import a ChatGPT or Claude export")
            .add_filter("Export", &["zip", "json"]),
    )
    .await?
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || crate::chat::import::import(&dir, &path))
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
        .map_err(String::from)
}

/// Writes a conversation as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_conversation(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    crate::chat::traces::signal(&config_dir(&app)?, &id, None, "exported");
    let text = conversation_markdown(&conversation);
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(format!("{}.md", sanitize_file_name(&conversation.title)))
            .add_filter("Markdown", &["md"]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Writes a canvas as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_canvas(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let canvas = crate::chat::canvas::load(&config_dir(&app)?, &id)?;
    let sheet = canvas.kind == "sheet";
    let web_app = canvas.kind == "app";
    let dialog = app.dialog().file();
    let multi_file = web_app && crate::chat::canvas::app_files(canvas.content()).is_some();
    let dialog = if multi_file {
        // Its files, as a ZIP to unpack and open index.html.
        dialog
            .set_file_name(format!("{}.zip", sanitize_file_name(&canvas.title)))
            .add_filter("ZIP archive", &["zip"])
    } else if web_app {
        // A single HTML file that runs in any browser.
        dialog
            .set_file_name(format!("{}.html", sanitize_file_name(&canvas.title)))
            .add_filter("Web page", &["html"])
    } else if sheet {
        dialog
            .set_file_name(format!("{}.xlsx", sanitize_file_name(&canvas.title)))
            .add_filter("Excel workbook", &["xlsx"])
            .add_filter("CSV", &["csv"])
    } else {
        dialog
            .set_file_name(format!("{}.md", sanitize_file_name(&canvas.title)))
            .add_filter("Markdown", &["md"])
    };
    let Some(path) = crate::support::dialogs::save_file(dialog).await? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    if sheet {
        let sheets = crate::chat::sheets::split_sheets(canvas.content(), &canvas.title);
        let csv = sheets.first().map(|(_, c)| c.clone()).unwrap_or_default();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            crate::chat::at_rest::write(&path, csv).map_err(|e| e.to_string())?;
        } else {
            // Every sheet into one workbook; CSV holds the first only.
            let rows: Vec<(String, Vec<Vec<String>>)> = sheets
                .iter()
                .map(|(n, c)| (n.clone(), crate::chat::sheets::parse_csv(c)))
                .collect();
            crate::chat::sheets::write_workbook(&path.with_extension("xlsx"), &rows)?;
        }
        return Ok(true);
    }
    if multi_file {
        crate::chat::at_rest::write(path, crate::chat::canvas::app_zip(canvas.content())?)
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if web_app {
        crate::chat::at_rest::write(path, crate::chat::canvas::app_html(canvas.content()))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    crate::chat::at_rest::write(path, canvas.content()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// A diagnostics file for a bug report: versions, platform, settings and counts, never what was
/// said, typed or stored. Saved where the user picks.
#[tauri::command]
pub async fn chat_export_diagnostics(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let dir = config_dir(&app)?;
    let providers = crate::chat::providers::load(&dir);
    let enabled: Vec<&String> = providers
        .providers
        .iter()
        .filter(|(_, p)| p.enabled)
        .map(|(k, _)| k)
        .collect();
    let report = serde_json::json!({
        "app": { "name": "Cortex", "version": env!("CARGO_PKG_VERSION") },
        "platform": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "family": std::env::consts::FAMILY },
        "providersEnabled": enabled,
        "defaultModel": providers.default_model,
        "counts": {
            "conversations": store::list_conversations(&dir).len(),
            "libraries": crate::chat::libraries::list(&dir).len(),
            "agents": crate::chat::agents::load(&dir).len(),
            "tasks": crate::chat::tasks::load(&dir).len(),
            "customTools": crate::chat::custom_tools::list(&dir).len(),
        },
        "features": {
            "semanticSearch": crate::chat::embeddings::ready(&crate::chat::embeddings::load(&dir)),
            "localApi": crate::chat::api_server::load(&dir).enabled,
            "computerUse": crate::chat::computer::load(&dir).enabled,
            "sandboxLanguages": crate::chat::sandbox::languages(),
        },
        "localApiRecent": crate::chat::api_server::recent().into_iter().take(20).map(|e| serde_json::json!({ "endpoint": e.endpoint, "status": e.status, "ms": e.ms })).collect::<Vec<_>>(),
        "createdAt": chrono::Utc::now(),
    });
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name("cortex-diagnostics.json")
            .add_filter("JSON", &["json"]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Adds every page and document of a ZIP the user picks (a Notion export, a folder of notes)
/// to a library.
#[tauri::command]
pub async fn chat_import_archive(
    app: AppHandle,
    library_id: String,
) -> Result<Option<crate::chat::libraries::ArchiveReport>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = crate::support::dialogs::pick_file(
        app.dialog()
            .file()
            .set_title("Import an export (Notion, Confluence, notes)")
            .add_filter("ZIP archive", &["zip"]),
    )
    .await?
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || {
        let bytes = crate::chat::at_rest::read(&path).map_err(|e| e.to_string())?;
        crate::chat::libraries::import_archive(&dir, &library_id, &bytes)
    })
    .await
    .map_err(|e| e.to_string())?
    .map(Some)
}

/// Saves an agent, its notebook and the skills it uses as one file, to back up or share.
#[tauri::command]
pub async fn chat_export_agent(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let dir = config_dir(&app)?;
    let mut bundle = crate::chat::agents::bundle(&dir, &id)?;
    // The tool servers it uses travel with it, without their secrets.
    let account = crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT)?;
    let servers = crate::tools::connectors::load_mcp_servers(&app, &account).unwrap_or_else(|e| {
        log::warn!("the agent's tool servers could not be read: {e}");
        Vec::new()
    });
    let servers: Vec<serde_json::Value> = servers
        .into_iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();
    bundle.mcp_servers = crate::chat::agents::shareable_mcp_servers(&bundle.agent, servers);
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(format!(
                "{}.cortex-agent.json",
                sanitize_file_name(&bundle.agent.name)
            ))
            .add_filter("Agent", &["json"]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Writes a conversation as JSON, every message and tool step included, to a file the user picks.
#[tauri::command]
pub async fn chat_export_conversation_json(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    let view = store::view(&conversation);
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(format!("{}.json", sanitize_file_name(&conversation.title)))
            .add_filter("JSON", &["json"]),
    )
    .await?
    else {
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
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Exports everything to a folder the user picks: each conversation as Markdown, and a copy of
/// the app's data (conversations, projects, agents, tasks, knowledge, skills, libraries).
#[tauri::command]
pub async fn chat_export_all(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(target) =
        crate::support::dialogs::pick_folder(app.dialog().file().set_title("Export to folder"))
            .await?
    else {
        return Ok(None);
    };
    let target = target.into_path().map_err(|e| e.to_string())?;
    let dir = config_dir(&app)?;
    let out = target.join(format!(
        "cortex-export-{}",
        chrono::Local::now().format("%Y-%m-%d-%H%M")
    ));
    let markdown = out.join("conversations");
    std::fs::create_dir_all(&markdown).map_err(|e| e.to_string())?;
    let mut count = 0;
    for summary in store::list_conversations(&dir) {
        let Ok(conversation) = store::load_conversation(&dir, &summary.id) else {
            continue;
        };
        let name = format!(
            "{} {}.md",
            summary.updated_at.format("%Y-%m-%d"),
            sanitize_file_name(&summary.title)
        );
        crate::chat::at_rest::write(markdown.join(name), conversation_markdown(&conversation))
            .map_err(|e| e.to_string())?;
        count += 1;
    }
    copy_dir(&store::chat_dir(&dir), &out.join("data/chat"))?;
    copy_dir(
        &crate::chat::skills::skills_dir(&dir),
        &out.join("data/skills"),
    )?;
    Ok(Some(format!(
        "Exported {count} conversations to {}",
        out.display()
    )))
}

/// Saves a conversation the page rendered as one self-contained web page, to share.
#[tauri::command]
pub async fn chat_save_shared_page(
    app: AppHandle,
    title: String,
    html: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    if html.len() > 64 * 1024 * 1024 {
        return Err("The conversation is too large to save as one page.".into());
    }
    let Some(path) = crate::support::dialogs::save_file(
        app.dialog()
            .file()
            .set_file_name(format!("{}.html", sanitize_file_name(&title)))
            .add_filter("Web page", &["html"]),
    )
    .await?
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path.with_extension("html"), html).map_err(|e| e.to_string())?;
    Ok(true)
}
