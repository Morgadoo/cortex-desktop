//! Conversations: listing, sending, steering, editing, rating, branching, deleting exchanges, the trash, and completing the message being typed.

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
    let dir = config_dir(&app)?;
    let conversation = store::load_conversation(&dir, &id)?;
    let running = state.is_running(&id);
    Ok(OpenConversation {
        view: store::view(&conversation),
        running,
        interrupted: (!running)
            .then(|| crate::chat::partials::get(&dir, &id))
            .flatten(),
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
pub fn chat_set_conversation(app: AppHandle, update: UpdateConversation) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &update.id)?;
    if let Some(title) = update.title {
        conversation.title = crate::support::text::clip(title.trim(), 120);
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
    store::save_conversation(&dir, &conversation).map_err(String::from)
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
    store::delete_conversation(&config_dir(&app)?, &id).map_err(String::from)
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

/// Keeps a temporary conversation, such as the answer chosen in a model comparison.
#[tauri::command]
pub fn chat_keep_conversation(app: AppHandle, id: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    conversation.temporary = false;
    store::save_conversation(&dir, &conversation).map_err(String::from)
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
    store::save_conversation(&dir, &conversation).map_err(String::from)
}

/// A conversation as plain text, to add to another one as context.
#[tauri::command]
pub fn chat_conversation_text(app: AppHandle, id: String) -> Result<String, String> {
    let conversation = store::load_conversation(&config_dir(&app)?, &id)?;
    Ok(crate::chat::store::chat_transcript(&conversation, 60_000))
}

/// Bookmarks a message (the turn ending at `index`) or removes its bookmark.
#[tauri::command]
pub fn chat_bookmark(app: AppHandle, id: String, index: usize, on: bool) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut conversation = store::load_conversation(&dir, &id)?;
    store::set_bookmark(&mut conversation, index, on)?;
    store::save_conversation(&dir, &conversation).map_err(String::from)
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
    let reason: String = crate::support::text::clip(reason.trim(), 500);
    conversation.rating_reasons.retain(|(i, _)| *i != index);
    // The reason also joins the rating in the trace, where the learning loop reads signals.
    crate::chat::traces::feedback_reason(&dir, &id, index, &reason);
    if !reason.is_empty() {
        conversation.rating_reasons.push((index, reason));
    }
    store::save_conversation(&dir, &conversation).map_err(String::from)
}

#[tauri::command]
pub fn chat_restore_conversation(app: AppHandle, id: String) -> Result<(), String> {
    crate::chat::backup::restore(&config_dir(&app)?, &id).map_err(String::from)
}

/// Replaces a damaged conversation with its newest backup; returns that backup's date.
#[tauri::command]
pub fn chat_recover_conversation(app: AppHandle, id: String) -> Result<String, String> {
    crate::chat::backup::recover(&config_dir(&app)?, &id).map_err(String::from)
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

/// What a running turn has streamed so far, for a window that opens during it.
#[tauri::command]
pub fn chat_live_events(
    state: State<'_, ChatState>,
    conversation_id: String,
) -> Vec<serde_json::Value> {
    state.replay(&conversation_id)
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

/// Deletes one conversation from the trash for good, or empties it.
#[tauri::command]
pub fn chat_delete_forever(app: AppHandle, id: Option<String>) -> Result<(), String> {
    crate::chat::backup::delete_forever(&config_dir(&app)?, id.as_deref()).map_err(String::from)
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
        Err(e) if e.to_string().contains("no text") => return Ok(String::new()),
        Err(e) => return Err(e.into()),
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
