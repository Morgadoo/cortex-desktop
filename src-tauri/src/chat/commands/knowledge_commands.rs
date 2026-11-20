//! What the assistant knows and how it works: memories, knowledge pages, instructions, projects, skills, plugins, learning, and the artifact registry.

use super::*;

/// What the assistant made for itself and how it is used: the artifact registry.
#[tauri::command]
pub fn chat_artifacts(app: AppHandle) -> Result<Vec<ScoredArtifact>, String> {
    let now = chrono::Utc::now();
    Ok(crate::chat::traces::artifacts(&config_dir(&app)?, None)
        .into_iter()
        .map(|artifact| ScoredArtifact {
            score: cortex_kernel::lifecycle::score(&artifact, now),
            artifact,
        })
        .collect())
}

/// Takes back the network and file access of a tool the assistant wrote.
#[tauri::command]
pub fn chat_revoke_tool(app: AppHandle, name: String) -> Result<(), String> {
    crate::chat::script_tools::revoke(&config_dir(&app)?, &name)
}

/// What changed from one saved version of an item to the next (or to `current`).
#[tauri::command]
pub fn chat_version_diff(
    app: AppHandle,
    kind: String,
    id: String,
    version: String,
    current: Option<serde_json::Value>,
) -> Result<String, String> {
    crate::chat::history::diff(&config_dir(&app)?, &kind, &id, &version, current.as_ref())
}

/// Brings back an earlier version of a memory list, knowledge pages, instructions, a notebook,
/// an agent, a workflow or a tool; the version it replaces is kept.
#[tauri::command]
pub fn chat_restore_version(
    app: AppHandle,
    kind: String,
    id: String,
    version: String,
) -> Result<(), String> {
    crate::chat::history::restore(&config_dir(&app)?, &kind, &id, &version)
}

/// The user's eval suites and their cases: the user owns them; no tool writes them.
#[tauri::command]
pub fn chat_eval_suites(app: AppHandle) -> Result<Vec<crate::chat::evals::SuiteView>, String> {
    Ok(crate::chat::evals::suite_views(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_eval_case(
    app: AppHandle,
    suite: String,
    case: crate::chat::evals::Case,
) -> Result<Vec<crate::chat::evals::SuiteView>, String> {
    let dir = config_dir(&app)?;
    crate::chat::evals::save_case(&dir, &suite, case)?;
    Ok(crate::chat::evals::suite_views(&dir))
}

#[tauri::command]
pub fn chat_delete_eval_case(
    app: AppHandle,
    suite: String,
    id: String,
) -> Result<Vec<crate::chat::evals::SuiteView>, String> {
    let dir = config_dir(&app)?;
    crate::chat::evals::delete_case(&dir, &suite, &id)?;
    Ok(crate::chat::evals::suite_views(&dir))
}

/// Marks an artifact critical, so the lifecycle never retires it, however rarely it is used.
#[tauri::command]
pub fn chat_set_artifact_critical(
    app: AppHandle,
    kind: String,
    id: String,
    critical: bool,
) -> Result<(), String> {
    let dir = config_dir(&app)?;
    if crate::chat::traces::artifact(&dir, &kind, &id).is_none() {
        return Err("There is no such artifact.".into());
    }
    crate::chat::traces::set_critical(&dir, &kind, &id, critical);
    Ok(())
}

#[tauri::command]
pub fn chat_knowledge_pages(app: AppHandle) -> Result<Vec<crate::chat::knowledge::Page>, String> {
    Ok(crate::chat::knowledge::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_knowledge_page(
    app: AppHandle,
    id: Option<String>,
    title: String,
    body: String,
) -> Result<Vec<crate::chat::knowledge::Page>, String> {
    let dir = config_dir(&app)?;
    crate::chat::knowledge::write(&dir, id.as_deref(), &title, &body, "user", &[])?;
    Ok(crate::chat::knowledge::load(&dir))
}

#[tauri::command]
pub fn chat_delete_knowledge_page(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::knowledge::Page>, String> {
    crate::chat::knowledge::delete(&config_dir(&app)?, &id)
}

/// The skill catalogues' skills (Anthropic's, and any added).
#[tauri::command]
pub async fn chat_skill_catalog(
    app: AppHandle,
    refresh: Option<bool>,
) -> Result<serde_json::Value, String> {
    let b =
        crate::chat::skill_catalog::browse(&config_dir(&app)?, refresh.unwrap_or(false)).await?;
    Ok(serde_json::json!({ "catalogs": b.catalogs, "listings": b.listings, "errors": b.errors }))
}

#[tauri::command]
pub async fn chat_add_skill_catalog(app: AppHandle, repo: String) -> Result<(), String> {
    crate::chat::skill_catalog::add_catalog(&config_dir(&app)?, &repo).await
}

#[tauri::command]
pub fn chat_remove_skill_catalog(app: AppHandle, repo: String) -> Result<(), String> {
    crate::chat::skill_catalog::remove_catalog(&config_dir(&app)?, &repo)
}

/// Adds a catalogue skill; says how many of its files came along.
#[tauri::command]
pub async fn chat_install_catalog_skill(
    app: AppHandle,
    catalog: String,
    branch: String,
    path: String,
) -> Result<serde_json::Value, String> {
    let done =
        crate::chat::skill_catalog::install(&config_dir(&app)?, &catalog, &branch, &path).await?;
    Ok(serde_json::json!({ "name": done.name, "files": done.files, "note": done.note }))
}

/// Shows another answer to the last prompt.
#[tauri::command]
pub fn chat_select_version(
    app: AppHandle,
    state: State<'_, ChatState>,
    id: String,
    from: usize,
    index: usize,
) -> Result<(), String> {
    if state.is_running(&id) {
        return Err("Wait for the reply to finish.".into());
    }
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::select_version(&mut conversation, from, index)?;
    store::save_conversation(&dir, &conversation).map_err(String::from)
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

/// Every version's answer at a branch point, to compare them.
#[tauri::command]
pub fn chat_version_texts(app: AppHandle, id: String, from: usize) -> Result<Vec<String>, String> {
    store::version_texts(&store::load_conversation(&config_dir(&app)?, &id)?, from)
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
        &engine::Context {
            instructions: Some(&instructions),
            ..Default::default()
        },
        &[],
    ))
    .max_tokens(400u64);
    let response = tokio::time::timeout(std::time::Duration::from_secs(60), model.call(request))
        .await
        .map_err(|_| "The provider did not answer in time.".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(response.text())
}

#[tauri::command]
pub fn chat_get_memories(app: AppHandle) -> Result<crate::chat::memory::Memories, String> {
    Ok(crate::chat::memory::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_add_memory(
    app: AppHandle,
    text: String,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    crate::chat::memory::add(&mut memories, &text, "user")?;
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_update_memory(
    app: AppHandle,
    id: String,
    text: String,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    let text = text.trim();
    if text.is_empty() || text.chars().count() > crate::chat::memory::MAX_LENGTH {
        return Err(format!(
            "A memory is 1 to {} characters.",
            crate::chat::memory::MAX_LENGTH
        ));
    }
    let memory = memories
        .items
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or("No such memory.")?;
    memory.text = text.to_string();
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

/// Files a memory: its category, importance (1–3) and whether the user confirmed it.
#[tauri::command]
pub fn chat_file_memory(
    app: AppHandle,
    id: String,
    category: String,
    importance: u8,
    confirmed: bool,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    let memory = memories
        .items
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or("No such memory.")?;
    let category = category.trim().to_lowercase();
    memory.category = if crate::chat::memory::CATEGORIES.contains(&category.as_str()) {
        category
    } else {
        String::new()
    };
    memory.importance = importance.clamp(1, 3);
    if confirmed {
        memory.confidence = "stated".into();
    }
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_delete_memory(
    app: AppHandle,
    id: Option<String>,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    match id {
        Some(id) => {
            crate::chat::memory::remove(&mut memories, &id);
        }
        None => memories.items.clear(),
    }
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

/// Whether the assistant may save and change memories and knowledge pages (Onyx's "update").
#[tauri::command]
pub fn chat_set_memory_update(
    app: AppHandle,
    update: bool,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    memories.update = update;
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_set_memory_enabled(
    app: AppHandle,
    enabled: bool,
) -> Result<crate::chat::memory::Memories, String> {
    let dir = config_dir(&app)?;
    let mut memories = crate::chat::memory::load(&dir);
    memories.enabled = enabled;
    crate::chat::memory::save(&dir, &memories)?;
    Ok(memories)
}

#[tauri::command]
pub fn chat_list_skills(app: AppHandle) -> Result<Vec<crate::chat::skills::Skill>, String> {
    Ok(crate::chat::skills::list(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_save_skill(
    app: AppHandle,
    name: String,
    description: String,
    body: String,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::save(&dir, name.trim(), &description, &body)?;
    Ok(crate::chat::skills::list(&dir))
}

#[tauri::command]
pub fn chat_delete_skill(
    app: AppHandle,
    name: String,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::delete(&dir, &name)?;
    Ok(crate::chat::skills::list(&dir))
}

#[tauri::command]
pub fn chat_set_skill_enabled(
    app: AppHandle,
    name: String,
    enabled: bool,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::set_enabled(&dir, &name, enabled)?;
    Ok(crate::chat::skills::list(&dir))
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
    crate::chat::skills::import(&config_dir(&app)?, &path).map(Some)
}

/// Adds files the user picks to a skill's folder (under `folder`, when given); returns the skills.
#[tauri::command]
pub async fn chat_skill_add_files(
    app: AppHandle,
    name: String,
    folder: Option<String>,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Add files to the skill")
        .pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    let dir = config_dir(&app)?;
    let folder = folder
        .map(|f| f.trim().trim_matches('/').to_string())
        .filter(|f| !f.is_empty());
    for path in rx.await.map_err(|e| e.to_string())?.unwrap_or_default() {
        let path = path.into_path().map_err(|e| e.to_string())?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel = folder
            .as_ref()
            .map_or_else(|| file_name.clone(), |f| format!("{f}/{file_name}"));
        let bytes = std::fs::read(&path).map_err(|e| format!("could not read {file_name}: {e}"))?;
        crate::chat::skills::add_file(&dir, &name, &rel, &bytes)?;
    }
    Ok(crate::chat::skills::list(&dir))
}

/// A skill's supporting file, as text.
#[tauri::command]
pub fn chat_skill_read_file(app: AppHandle, name: String, path: String) -> Result<String, String> {
    crate::chat::skills::read_file(&config_dir(&app)?, &name, &path)
}

/// Writes a text file into a skill's folder (a new file, or an edit).
#[tauri::command]
pub fn chat_skill_write_file(
    app: AppHandle,
    name: String,
    path: String,
    text: String,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::add_file(&dir, &name, path.trim(), text.as_bytes())?;
    Ok(crate::chat::skills::list(&dir))
}

#[tauri::command]
pub fn chat_skill_remove_file(
    app: AppHandle,
    name: String,
    path: String,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::remove_file(&dir, &name, &path)?;
    Ok(crate::chat::skills::list(&dir))
}

/// Imports a skill from a link (a GitHub folder or a SKILL.md); returns its name.
#[tauri::command]
pub async fn chat_import_skill_url(app: AppHandle, url: String) -> Result<String, String> {
    crate::chat::skills::import_from_link(&config_dir(&app)?, &url).await
}

/// The learning switches (weekly knowledge review, skills from work); `change` sets them.
#[tauri::command]
pub fn chat_learning(
    app: AppHandle,
    change: Option<crate::chat::learning::Learning>,
) -> Result<crate::chat::learning::Learning, String> {
    let dir = config_dir(&app)?;
    if let Some(mut next) = change {
        next.last_review = crate::chat::learning::load(&dir).last_review;
        crate::chat::learning::save(&dir, &next)?;
    }
    Ok(crate::chat::learning::load(&dir))
}

/// Earlier versions of an agent, workflow or tool, newest first.
#[tauri::command]
pub fn chat_versions(
    app: AppHandle,
    kind: String,
    id: String,
) -> Result<Vec<crate::chat::versions::Version>, String> {
    crate::chat::versions::list(&config_dir(&app)?, &kind, &id)
}

/// The plugins of a Claude plugin marketplace on GitHub, to choose from; `None` when the link is
/// a plugin rather than a marketplace.
#[tauri::command]
pub async fn chat_plugin_marketplace(
    link: String,
) -> Result<Option<Vec<crate::chat::plugins::MarketEntry>>, String> {
    crate::chat::plugins::read_market(&link).await
}

/// Imports Claude plugins (or a marketplace of them) from a GitHub link, or from a folder the
/// user picks when no link is given: their skills, commands, agents and tool servers.
#[tauri::command]
pub async fn chat_import_plugin(
    app: AppHandle,
    link: Option<String>,
) -> Result<Option<crate::chat::plugins::Imported>, String> {
    use crate::chat::plugins;
    let dir = config_dir(&app)?;
    let files = match link.map(|l| l.trim().to_string()).filter(|l| !l.is_empty()) {
        Some(link) => {
            let (urls, folder) = plugins::github_sources(&link)?;
            let client = crate::support::http::client(std::time::Duration::from_secs(120))?;
            let mut last = "The repository could not be downloaded.".to_string();
            let mut found = None;
            for url in urls {
                match client.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let bytes = r.bytes().await.map_err(|e| e.to_string())?;
                        found = Some(plugins::subfolder(plugins::from_zip(&bytes)?, &folder));
                        break;
                    }
                    Ok(r) => last = format!("GitHub answered {} for {url}.", r.status()),
                    Err(e) => last = e.to_string(),
                }
            }
            found.ok_or(last)?
        }
        None => {
            use tauri_plugin_dialog::DialogExt;
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.dialog()
                .file()
                .set_title("Choose a Claude plugin or marketplace folder")
                .pick_folder(move |path| {
                    let _ = tx.send(path);
                });
            let Some(path) = rx.await.map_err(|e| e.to_string())? else {
                return Ok(None);
            };
            let path = path.into_path().map_err(|e| e.to_string())?;
            tokio::task::spawn_blocking(move || plugins::from_dir(&path))
                .await
                .map_err(|e| e.to_string())??
        }
    };
    let roots = plugins::plugin_roots(&files);
    if roots.is_empty() {
        return Err("No Claude plugin was found there: it needs skills/, commands/, agents/, .mcp.json or .claude-plugin/.".into());
    }
    let account = crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT)?;
    let existing = crate::tools::connectors::load_mcp_servers(&app, &account).unwrap_or_default();
    let mut out = plugins::Imported::default();
    for root in roots {
        let plugin = plugins::read_plugin(&files, &root);
        log::info!(
            "importing the plugin {} ({})",
            plugin.name,
            plugin.description
        );
        plugins::install(&dir, &plugin, &mut out)?;
        if plugin.servers.is_empty() && plugin.hooks.is_none() {
            continue;
        }
        // Servers and hooks can run the plugin's own scripts, so its files are kept on disk.
        let kept = plugins::plugin_dir(&dir, &plugin.name);
        plugins::keep_files(&files, &plugin.root, &kept)?;
        for server in &plugin.servers {
            let Some(config) = plugins::server_config(&plugin.name, server, &kept) else {
                out.skipped.push(format!("tool server {}", server.name));
                continue;
            };
            if existing.iter().any(|s| s.id == config.id) {
                out.skipped
                    .push(format!("tool server {} (already added)", server.name));
                continue;
            }
            let name = config.display_name.clone();
            match crate::tools::commands::save_mcp_server(
                config,
                app.clone(),
                app.state(),
                app.state(),
            )
            .await
            {
                Ok(()) => out.servers.push(name),
                Err(e) => out.skipped.push(format!("tool server {name}: {e}")),
            }
        }
    }
    Ok(Some(out))
}

#[tauri::command]
pub fn chat_skill_history(
    app: AppHandle,
    name: String,
) -> Result<Vec<crate::chat::skills::Version>, String> {
    crate::chat::skills::history(&config_dir(&app)?, &name)
}

#[tauri::command]
pub fn chat_restore_skill(
    app: AppHandle,
    name: String,
    version: String,
) -> Result<Vec<crate::chat::skills::Skill>, String> {
    let dir = config_dir(&app)?;
    crate::chat::skills::restore(&dir, &name, &version)?;
    Ok(crate::chat::skills::list(&dir))
}
