//! Conversations: sending, steering, editing, rating, branching, exporting and searching chats.

use super::*;

#[tauri::command]
pub fn chat_list_conversations(app: AppHandle) -> Result<Vec<ConversationSummary>, String> {
    Ok(store::list_conversations(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_get_conversation(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
) -> Result<OpenConversation, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    Ok(OpenConversation {
        view: store::view(&conversation),
        running: state.is_running(&id),
    })
}

#[tauri::command]
pub fn chat_send(
    app: AppHandle,
    state: State<'_, ChatState>,
    request: TurnRequest,
) -> Result<(), String> {
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
        // A rated-down answer waits in the regressions suite for the user's expectations.
        if rating < 0 {
            if let Err(e) = crate::chat::evals::regression_from_rating(&dir, &conversation, index) {
                log::warn!("could not keep a regression case: {e}");
            }
        }
        crate::chat::traces::feedback(
            &dir,
            &conversation.id,
            index,
            rating.signum() as i64,
            conversation.model.as_deref(),
            conversation.agent_id.as_deref(),
        );
    }
    store::save_conversation(&dir, &conversation)
}

/// Something the user did with an answer (`copied`), recorded as a signal in the trace.
#[tauri::command]
pub fn chat_signal(
    app: AppHandle,
    id: String,
    index: Option<usize>,
    kind: String,
) -> Result<(), String> {
    crate::chat::traces::signal(&config_dir(&app)?, &id, index, &kind);
    Ok(())
}

#[tauri::command]
pub fn chat_delete_conversation(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
) -> Result<(), String> {
    state.cancel(&id);
    store::delete_conversation(&config_dir(&app)?, &id)
}

/// Drafts a workflow from a description, or revises `current` as `request` asks.
#[tauri::command]
pub async fn chat_draft_workflow(
    app: AppHandle,
    request: String,
    current: Option<crate::chat::workflows::Workflow>,
) -> Result<crate::chat::workflows::Workflow, String> {
    use rig_core::completion::CompletionRequest;
    let dir = config_dir(&app)?;
    if request.trim().len() < 4 {
        return Err("Say what the workflow is for, or what to change.".into());
    }
    let reference = providers::load(&dir)
        .default_model
        .ok_or("Choose a default model under Providers & models first.")?;
    let prompt = crate::chat::workflows::draft_prompt(
        request.trim(),
        current
            .as_ref()
            .filter(|w| !w.steps.is_empty() || !w.name.is_empty()),
    );
    let text = engine::ask_text(
        &dir,
        &reference,
        CompletionRequest::new(rig_core::completion::Message::user(prompt)),
        120,
    )
    .await?;
    let mut draft = crate::chat::workflows::parse_draft(&text)?;
    if let Some(c) = current {
        draft.id = c.id;
        draft.builtin = c.builtin;
    }
    Ok(draft)
}

/// Adds guidance to a running turn; it joins the conversation at the turn's next step.
#[tauri::command]
pub fn chat_steer(
    state: State<'_, ChatState>,
    conversation_id: String,
    text: String,
) -> Result<bool, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(false);
    }
    Ok(state.steer(&conversation_id, text))
}

#[tauri::command]
pub fn chat_cancel_local_model(id: String) {
    crate::chat::local_models::cancel(&id);
}

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
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!(
            "cortex-activity-{}.{ext}",
            chrono::Local::now().format("%Y-%m-%d")
        ))
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(target) = rx.await.map_err(|e| e.to_string())? else {
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
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Where to write the catalogue")
        .pick_folder(move |p| {
            let _ = tx.send(p);
        });
    let Some(folder) = rx.await.map_err(|e| e.to_string())? else {
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
    tokio::task::spawn_blocking(move || crate::chat::import::import(&dir, &path))
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
}

/// Keeps a temporary conversation, such as the answer chosen in a model comparison.
#[tauri::command]
pub fn chat_keep_conversation(app: AppHandle, id: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    conversation.temporary = false;
    store::save_conversation(&dir, &conversation)
}

/// Three short messages the user might send next, from the latest exchange (Open WebUI's
/// follow-up prompts). Empty when the provider gives nothing usable.
#[tauri::command]
pub async fn chat_follow_ups(
    app: AppHandle,
    id: String,
    model: String,
) -> Result<Vec<String>, String> {
    let dir = config_dir(&app)?;
    let conversation = store::load_conversation(&dir, &id)?;
    let turns = store::turns(&conversation);
    let text_of = |t: &store::Turn| {
        t.parts
            .iter()
            .filter_map(|p| {
                if let store::Part::Text { text } = p {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let Some(answer) = turns
        .iter()
        .rev()
        .find(|t| t.role == "assistant")
        .map(text_of)
        .filter(|t| !t.trim().is_empty())
    else {
        return Ok(Vec::new());
    };
    let question = turns
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map(text_of)
        .unwrap_or_default();
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(follow_up_prompt(&question, &answer)),
    )
    .max_tokens(1_000u64);
    Ok(parse_follow_ups(
        &engine::ask_text(&dir, &model, request, 30).await?,
    ))
}

/// Writes a conversation as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_conversation(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    crate::chat::traces::signal(&config_dir(&app)?, &id, None, "exported");
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
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Turns the finished recording into a skill draft for review; the screenshots are dropped then.
#[tauri::command]
pub async fn chat_record_draft(
    app: AppHandle,
    recorder: tauri::State<'_, crate::chat::recorder::Recorder>,
    model: String,
    note: String,
) -> Result<crate::chat::recorder::Draft, String> {
    let frames = recorder.frames();
    let draft = crate::chat::recorder::draft(&config_dir(&app)?, &model, &frames, &note).await?;
    recorder.discard();
    Ok(draft)
}

/// Translates the interface text still missing for `code` with `model`.
#[tauri::command]
pub async fn chat_translate_ui(
    app: AppHandle,
    model: String,
    code: String,
) -> Result<crate::chat::ui_language::Coverage, String> {
    crate::chat::ui_language::translate(&config_dir(&app)?, &model, &code).await
}

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
    Ok(crate::chat::share::links(&config_dir(&app)?).await)
}

#[tauri::command]
pub fn chat_stop_share(app: AppHandle, token: String) -> Result<(), String> {
    crate::chat::share::stop(&config_dir(&app)?, &token)
}

/// Vibe's "Sensitive data" switch for knowledge.
#[tauri::command]
pub fn chat_set_keep_sensitive_out(
    app: AppHandle,
    on: bool,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    memories.keep_sensitive_out = on;
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

/// Writes a canvas as Markdown to a file the user picks.
#[tauri::command]
pub async fn chat_export_canvas(app: AppHandle, id: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let canvas = crate::chat::canvas::load(&config_dir(&app)?, &id)?;
    let sheet = canvas.kind == "sheet";
    let web_app = canvas.kind == "app";
    let (tx, rx) = tokio::sync::oneshot::channel();
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
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
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
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name("cortex-diagnostics.json")
        .add_filter("JSON", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
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
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Import an export (Notion, Confluence, notes)")
        .add_filter("ZIP archive", &["zip"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
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

/// Rewrites a message (yours or an answer) in place, without asking for a new reply.
#[tauri::command]
pub fn chat_rewrite_message(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
    index: usize,
    text: String,
) -> Result<(), String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish first.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::rewrite_message(&mut conversation, index, &text)?;
    store::save_conversation(&dir, &conversation)
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
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!(
            "{}.cortex-agent.json",
            sanitize_file_name(&bundle.agent.name)
        ))
        .add_filter("Agent", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
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
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// An answer translated into `language`, Markdown kept; shown beside the original, not stored.
#[tauri::command]
pub async fn chat_translate(
    app: AppHandle,
    model: String,
    text: String,
    language: String,
) -> Result<String, String> {
    let language: String = language.trim().chars().take(40).collect();
    if language.is_empty() || text.trim().is_empty() {
        return Err("Nothing to translate.".into());
    }
    let prompt = format!(
        "Translate the text below into {language}. Keep its Markdown, code blocks, links and numbers exactly; \
         translate only the prose. Reply with the translation alone.\n\n{}",
        text.chars().take(30_000).collect::<String>()
    );
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(8_000u64);
    engine::ask_text(&config_dir(&app)?, &model, request, 120).await
}

/// Rewrites `text` as asked (shorter, clearer, fixed grammar, …) for editing in place. `context`
/// is the text around it, so a rewritten passage still fits.
#[tauri::command]
pub async fn chat_improve_text(
    app: AppHandle,
    model: String,
    text: String,
    instruction: String,
    context: Option<String>,
) -> Result<String, String> {
    let instruction: String = instruction.trim().chars().take(400).collect();
    if text.trim().is_empty() {
        return Err("Nothing to improve.".into());
    }
    let instruction = if instruction.is_empty() {
        "Improve the writing: clearer and tighter, same meaning.".to_string()
    } else {
        instruction
    };
    let context = context
        .filter(|c| !c.trim().is_empty())
        .map(|c| format!("\n\nThe text around it, for reference only (do not repeat it):\n<context>\n{}\n</context>", c.chars().take(12_000).collect::<String>()))
        .unwrap_or_default();
    let prompt = format!(
        "Rewrite the text below. Instruction: {instruction}\n\
         Keep its Markdown, links, code and facts; do not add facts. Reply with the rewritten text alone, \
         with no preamble or quotes.{context}\n\n<text>\n{}\n</text>",
        text.chars().take(30_000).collect::<String>()
    );
    let request = rig_core::completion::CompletionRequest::new(
        rig_core::completion::message::Message::user(prompt),
    )
    .max_tokens(8_000u64);
    let out = engine::ask_text(&config_dir(&app)?, &model, request, 120).await?;
    let out = out
        .trim()
        .trim_start_matches("<text>")
        .trim_end_matches("</text>")
        .trim();
    if out.is_empty() {
        return Err("The model gave no text back.".into());
    }
    Ok(out.to_string())
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

/// A conversation as plain text, to add to another one as context.
#[tauri::command]
pub fn chat_conversation_text(app: AppHandle, id: String) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    Ok(crate::chat::builtins::chat_transcript(
        &conversation,
        60_000,
    ))
}

/// Bookmarks a message (the turn ending at `index`) or removes its bookmark.
#[tauri::command]
pub fn chat_bookmark(app: AppHandle, id: String, index: usize, on: bool) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::set_bookmark(&mut conversation, index, on)?;
    store::save_conversation(&dir, &conversation)
}

/// Why a reply was rated down, kept with the conversation.
#[tauri::command]
pub fn chat_rate_reason(
    app: AppHandle,
    id: String,
    index: usize,
    reason: String,
) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    let reason: String = reason.trim().chars().take(500).collect();
    conversation.rating_reasons.retain(|(i, _)| *i != index);
    // The reason also joins the rating in the trace, where the learning loop reads signals.
    crate::chat::traces::feedback_reason(&dir, &id, index, &reason);
    if !reason.is_empty() {
        conversation.rating_reasons.push((index, reason));
    }
    store::save_conversation(&dir, &conversation)
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

/// Exports everything to a folder the user picks: each conversation as Markdown, and a copy of
/// the app's data (conversations, projects, agents, tasks, knowledge, skills, libraries).
#[tauri::command]
pub async fn chat_export_all(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export to folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    let Some(target) = rx.await.map_err(|e| e.to_string())? else {
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

/// Drafts an agent from a description, with the default model.
#[tauri::command]
pub async fn chat_draft_agent(
    app: AppHandle,
    description: String,
    current: Option<crate::chat::agents::Agent>,
) -> Result<crate::chat::agents::Agent, String> {
    use rig_core::completion::CompletionRequest;
    let dir = config_dir(&app)?;
    if description.trim().len() < 8 {
        return Err("Describe the agent in a sentence or two.".into());
    }
    let reference = providers::load(&dir)
        .default_model
        .ok_or("Choose a default model under Providers & models first.")?;
    let tools: Vec<&str> = TOOL_CHOICES.to_vec();
    let skills: Vec<String> = crate::chat::skills::list(&dir)
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| s.name)
        .collect();
    // With a draft already made, the description is a change to it (the builder is a conversation).
    let prompt = match &current {
        Some(agent) => {
            crate::chat::agents::refine_prompt(agent, description.trim(), &tools, &skills)
        }
        None => crate::chat::agents::draft_prompt(description.trim(), &tools, &skills),
    };
    let request = CompletionRequest::new(rig_core::completion::Message::user(prompt));
    let text = engine::ask_text(&dir, &reference, request, 120).await?;
    let mut agent = crate::chat::agents::parse_draft(&text, &tools, &skills)?;
    if let Some(c) = current {
        agent.id = c.id;
    }
    Ok(agent)
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
    crate::chat::at_rest::write(path.with_extension("html"), html).map_err(|e| e.to_string())?;
    Ok(true)
}

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
pub fn chat_restore_conversation(app: AppHandle, id: String) -> Result<(), String> {
    crate::chat::backup::restore(&config_dir(&app)?, &id)
}

/// Replaces a damaged conversation with its newest backup; returns that backup's date.
#[tauri::command]
pub fn chat_recover_conversation(app: AppHandle, id: String) -> Result<String, String> {
    crate::chat::backup::recover(&config_dir(&app)?, &id)
}

/// What fills the model's context window in a conversation.
#[tauri::command]
pub fn chat_context_breakdown(
    app: AppHandle,
    id: String,
) -> Result<crate::chat::context::Breakdown, String> {
    let dir = config_dir(&app)?;
    let c = store::load_conversation(&dir, &id)?;
    let reference = c
        .model
        .clone()
        .or_else(|| providers::load(&dir).default_model)
        .unwrap_or_default();
    Ok(crate::chat::context::breakdown(
        &c.messages,
        c.summary.as_ref(),
        c.usage.context_tokens,
        crate::chat::context::context_window(&reference),
        &c.prompt_parts,
    ))
}

/// Branches a conversation into a new one that continues from the same point.
#[tauri::command]
pub fn chat_fork_conversation(
    app: AppHandle,
    id: String,
    upto: Option<usize>,
    without_files: Option<bool>,
    agent_id: Option<String>,
) -> Result<String, String> {
    let dir = config_dir(&app)?;
    let mut c = store::load_conversation(&dir, &id)?;
    store::branch_copy(&mut c, upto, without_files.unwrap_or(false))?;
    // Handed to an agent (LobeHub's forward to another assistant): it carries on from here.
    let handed = agent_id.as_deref().is_some_and(|a| !a.is_empty());
    if let Some(agent) = agent_id.filter(|a| !a.is_empty()) {
        let found = crate::chat::agents::find(&dir, &agent).ok_or("There is no such agent.")?;
        c.agent_id = Some(found.id);
        c.project_id = None;
        c.title = format!(
            "{} ({})",
            if c.title.is_empty() {
                engine::fallback_title(&c)
            } else {
                c.title.clone()
            },
            found.name
        );
    }
    let title = if c.title.is_empty() {
        engine::fallback_title(&c)
    } else {
        c.title.clone()
    };
    c.id = uuid::Uuid::new_v4().to_string();
    c.title = if handed {
        title
    } else {
        format!("{title} (branch)")
    };
    c.pinned = false;
    c.unread = false;
    c.temporary = false;
    c.archived = false;
    c.task_id = None;
    // The branch has spent nothing yet; the original keeps what it cost (budgets count it once).
    c.usage = Default::default();
    c.created_at = chrono::Utc::now();
    c.updated_at = c.created_at;
    store::save_conversation(&dir, &c)?;
    Ok(c.id)
}
