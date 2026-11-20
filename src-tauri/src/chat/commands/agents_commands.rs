//! Agents and workflows: the team, its organization, templates, notebooks, the arena, saved and drafted workflows, and which version of each is in production.

use super::*;

/// Auto's fast and strong models.
#[tauri::command]
pub fn chat_router_models(app: AppHandle) -> Result<(Option<String>, Option<String>), String> {
    let file = crate::chat::providers::load(&config_dir(&app)?);
    Ok((file.router_fast, file.router_strong))
}

#[tauri::command]
pub fn chat_set_router_models(
    app: AppHandle,
    fast: Option<String>,
    strong: Option<String>,
) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut file = crate::chat::providers::load(&dir);
    file.router_fast = fast.filter(|m| !m.trim().is_empty() && m != crate::chat::router::AUTO);
    file.router_strong = strong.filter(|m| !m.trim().is_empty() && m != crate::chat::router::AUTO);
    crate::chat::providers::save(&dir, &file).map_err(String::from)
}

/// The community agent catalog (LobeHub's open index, or another with its layout).
#[tauri::command]
pub async fn chat_agent_market(
    app: AppHandle,
    source: Option<String>,
    language: Option<String>,
    refresh: Option<bool>,
) -> Result<Vec<crate::chat::agent_market::Listing>, String> {
    let source = source
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .unwrap_or_else(|| crate::chat::agent_market::DEFAULT_SOURCE.into());
    crate::chat::agent_market::listings(
        &config_dir(&app)?,
        &source,
        language.as_deref().unwrap_or("en"),
        refresh.unwrap_or(false),
    )
    .await
    .map_err(String::from)
}

/// Adds a catalog agent as one of the user's agents.
#[tauri::command]
pub async fn chat_install_market_agent(
    app: AppHandle,
    identifier: String,
    source: Option<String>,
    language: Option<String>,
) -> Result<crate::chat::agents::Agent, String> {
    let source = source
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .unwrap_or_else(|| crate::chat::agent_market::DEFAULT_SOURCE.into());
    crate::chat::agent_market::install(
        &config_dir(&app)?,
        &source,
        language.as_deref().unwrap_or("en"),
        &identifier,
    )
    .await
    .map_err(String::from)
}

#[tauri::command]
pub fn chat_list_agents(app: AppHandle) -> Result<Vec<crate::chat::agents::Agent>, String> {
    Ok(crate::chat::agents::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn chat_agent_templates() -> Vec<crate::chat::agents::Agent> {
    crate::chat::agents::templates()
}

#[tauri::command]
pub fn chat_save_agent(
    app: AppHandle,
    agent: crate::chat::agents::Agent,
) -> Result<Vec<crate::chat::agents::Agent>, String> {
    crate::chat::agents::save(&config_dir(&app)?, agent).map_err(String::from)
}

#[tauri::command]
pub fn chat_delete_agent(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::chat::agents::Agent>, String> {
    crate::chat::agents::delete(&config_dir(&app)?, &id).map_err(String::from)
}

/// Whether new chats go to the best-suited agent by themselves.
#[tauri::command]
pub fn chat_agent_routing(app: AppHandle, enabled: Option<bool>) -> Result<bool, String> {
    let dir = config_dir(&app)?;
    if let Some(on) = enabled {
        crate::chat::agents::set_routing(&dir, on)?;
    }
    Ok(crate::chat::agents::routing_enabled(&dir))
}

/// The agent's own files library (made on first use).
#[tauri::command]
pub fn chat_agent_files(
    app: AppHandle,
    id: String,
) -> Result<crate::chat::libraries::Library, String> {
    crate::chat::agents::files_library(&config_dir(&app)?, &id).map_err(String::from)
}

/// Opens an agent's browser profile in a window, to sign in to sites for it; `clear` forgets it.
#[tauri::command]
pub fn chat_agent_browser(
    app: AppHandle,
    agent_id: Option<String>,
    url: Option<String>,
    clear: bool,
) -> Result<(), String> {
    let dir = config_dir(&app)?;
    if clear {
        return crate::chat::webpage::clear_profile(&dir, agent_id.as_deref())
            .map_err(String::from);
    }
    crate::chat::webpage::open_profile(
        &dir,
        agent_id.as_deref(),
        url.as_deref().unwrap_or_default(),
    )
    .map_err(String::from)
}

/// Forgets an agent's code environment (its files and installed packages).
#[tauri::command]
pub fn chat_reset_agent_env(app: AppHandle, id: String) -> Result<(), String> {
    crate::chat::sandbox::reset_agent_env(&config_dir(&app)?, &id).map_err(String::from)
}

/// The user's ACP agents; `agents` replaces them. Returns every one that can start.
#[tauri::command]
pub fn chat_acp_agents(
    app: AppHandle,
    agents: Option<Vec<crate::chat::acp::AcpAgent>>,
) -> Result<Vec<crate::chat::acp::AcpAgent>, String> {
    let dir = config_dir(&app)?;
    if let Some(list) = agents {
        crate::chat::acp::save(&dir, &list)?;
    }
    Ok(crate::chat::acp::all(&dir))
}

/// Adds an agent from an exported file; returns its name, or `None` when cancelled.
#[tauri::command]
pub async fn chat_import_agent(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = crate::support::dialogs::pick_file(
        app.dialog()
            .file()
            .set_title("Import an agent")
            .add_filter("Agent", &["json"]),
    )
    .await?
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let text = crate::chat::at_rest::read_to_string(path).map_err(|e| e.to_string())?;
    let bundle: crate::chat::agents::Bundle = serde_json::from_str(&text)
        .map_err(|_| "This file is not an exported agent.".to_string())?;
    let servers = bundle.mcp_servers.clone();
    let agent = crate::chat::agents::install(&config_dir(&app)?, bundle)?;
    // Its tool servers are added when this computer does not have them yet; their keys are set
    // under Connectors.
    let account = crate::account::AccountId::parse(crate::chat::LOCAL_ACCOUNT)?;
    let existing = crate::tools::connectors::load_mcp_servers(&app, &account).unwrap_or_default();
    let mut added = Vec::new();
    for value in servers {
        let Ok(server) = serde_json::from_value::<crate::tools::connectors::McpServerConfig>(value)
        else {
            continue;
        };
        if existing.iter().any(|s| s.id == server.id) {
            continue;
        }
        let name = server.display_name.clone();
        match crate::tools::commands::save_mcp_server(server, app.clone(), app.state(), app.state())
            .await
        {
            Ok(()) => added.push(name),
            Err(e) => log::warn!("could not add the agent's tool server {name}: {e}"),
        }
    }
    Ok(Some(if added.is_empty() {
        agent.name
    } else {
        format!(
            "{} (with its tool servers {}; add their keys under Connectors)",
            agent.name,
            added.join(", ")
        )
    }))
}

/// Takes one recent file off the list, or all of them.
#[tauri::command]
pub fn chat_forget_recent_file(
    app: AppHandle,
    index: Option<usize>,
) -> Result<Vec<crate::chat::recent_files::RecentFile>, String> {
    crate::chat::recent_files::forget(&config_dir(&app)?, index).map_err(String::from)
}

/// Records which model's answer was kept in a comparison.
#[tauri::command]
pub fn chat_arena_record(
    app: AppHandle,
    winner: String,
    losers: Vec<String>,
) -> Result<(), String> {
    crate::chat::arena::record(&config_dir(&app)?, &winner, &losers).map_err(String::from)
}

#[tauri::command]
pub fn chat_arena_board(app: AppHandle) -> Result<Vec<crate::chat::arena::Standing>, String> {
    Ok(crate::chat::arena::standings(&crate::chat::arena::load(
        &config_dir(&app)?,
    )))
}

#[tauri::command]
pub fn chat_arena_reset(app: AppHandle) -> Result<(), String> {
    crate::chat::arena::reset(&config_dir(&app)?).map_err(String::from)
}

/// An agent's notebook.
#[tauri::command]
pub fn chat_agent_notebook(app: AppHandle, id: String) -> Result<String, String> {
    Ok(crate::chat::agents::notebook(&config_dir(&app)?, &id))
}

#[tauri::command]
pub fn chat_save_agent_notebook(app: AppHandle, id: String, text: String) -> Result<(), String> {
    crate::chat::agents::save_notebook(&config_dir(&app)?, &id, &text).map_err(String::from)
}

/// What each agent's conversations cost this month, by agent id.
#[tauri::command]
pub fn chat_agent_spend(
    app: AppHandle,
) -> Result<std::collections::HashMap<String, crate::chat::agents::Spend>, String> {
    Ok(crate::chat::agents::spend_this_month(&config_dir(&app)?))
}

/// What a sub-agent did, step by step.
#[tauri::command]
pub fn chat_subagent_transcript(
    app: AppHandle,
    call_id: String,
    index: usize,
) -> Result<Vec<crate::chat::subagents::Step>, String> {
    Ok(crate::chat::subagents::transcript(
        &config_dir(&app)?,
        &call_id,
        index,
    ))
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
        .map_err(String::from)
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
