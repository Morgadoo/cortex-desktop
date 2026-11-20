//! Agents talking to each other, and a message routed to the agent that should answer it.

use super::*;
use crate::chat::error::ChatError;

/// `message_agent`: the receiving agent answers in a new chat of its own (unread, so the user
/// sees it), and its reply comes back as the tool's result.
pub(super) async fn message_agent(
    host: &Arc<dyn Host>,
    config_dir: &std::path::Path,
    from_chat: &str,
    from: Option<&agents::Agent>,
    arguments: &Value,
    workdir: Option<&str>,
    approval: ApprovalMode,
) -> tools::Outcome {
    let fail = |text: String| tools::Outcome {
        text,
        is_error: true,
    };
    let s = |k: &str| {
        arguments
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let team = agents::load(config_dir);
    let Some(target) = agents::by_name(&team, &s("agent")).cloned() else {
        let names: Vec<&str> = team.iter().map(|a| a.name.as_str()).collect();
        return fail(format!(
            "There is no agent by that name. The agents are: {}.",
            names.join(", ")
        ));
    };
    if from.is_some_and(|f| f.id == target.id) {
        return fail("An agent cannot message itself.".into());
    }
    if s("message").is_empty() {
        return fail("Write the message.".into());
    }
    let paths: Vec<String> = arguments
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let files = match agents::read_shared(&paths, workdir.map(std::path::Path::new)) {
        Ok(f) => f,
        Err(e) => return fail(e.to_string()),
    };
    let Some(depth) = host.chat_state().enter_message(agents::MAX_MESSAGE_DEPTH) else {
        return fail(
            "Messages are already going back and forth; answer with what you have.".into(),
        );
    };
    let sender = from.map_or_else(|| "the assistant".to_string(), |a| a.name.clone());
    let model = target
        .model
        .clone()
        .or_else(|| {
            store::load_conversation(config_dir, from_chat)
                .ok()
                .and_then(|c| c.model)
        })
        .or(providers::load(config_dir).default_model);
    let result = async {
        let model = model
            .ok_or_else(|| ChatError::Invalid("No model to run the other agent on.".into()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut c = Conversation::new(id.clone(), None, Some(model.clone()));
        c.title = format!(
            "From {sender}: {}",
            s("message").chars().take(50).collect::<String>()
        );
        c.agent_id = Some(target.id.clone());
        c.unread = true;
        store::save_conversation(config_dir, &c)?;
        let cancel = host
            .chat_state()
            .begin(&id)
            .ok_or_else(|| ChatError::Invalid("That chat is busy.".into()))?;
        let request = TurnRequest {
            conversation_id: id.clone(),
            text: agents::message_text(&sender, &s("message"), &files),
            model,
            speed: if target.think {
                Speed::Think
            } else {
                Speed::Fast
            },
            // The sender's mode: the same person is (or is not) there to approve. From a chat in
            // Auto the other agent runs its code and makes its changes; from Ask it asks; from a
            // scheduled run nobody watches it stays unattended.
            approval,
            project_id: None,
            agent_id: Some(target.id.clone()),
            images: Vec::new(),
            edit_index: None,
            temporary: false,
            resume: false,
            workdir: None,
            connectors: None,
            route_note: None,
            tools_off: Vec::new(),
            max_steps: None,
            only_tools: None,
            system: None,
            skills_off: Vec::new(),
            facts_off: Vec::new(),
            pages_off: Vec::new(),
            instructions_off: false,
        };
        // A turn of its own, so the other agent's events stream to its chat as usual.
        queue_turn(host, request, cancel)
            .await
            .map_err(|_| "The other agent's turn stopped.".to_string())
            .map_err(ChatError::Invalid)?;
        let done = store::load_conversation(config_dir, &id)?;
        let reply = done
            .messages
            .last()
            .filter(|m| matches!(m, Message::Assistant(_)))
            .map(crate::chat::context::message_text)
            .unwrap_or_default();
        if reply.trim().is_empty() {
            return Err(ChatError::Invalid(format!(
                "{} could not answer.",
                target.name
            )));
        }
        Ok::<String, String>(format!(
            "{} replied:\n\n{reply}\n\n(The exchange is kept in a chat under {} in the sidebar.)",
            target.name, target.name
        ))
        .map_err(ChatError::Invalid)
    }
    .await;
    drop(depth);
    match result {
        Ok(text) => tools::Outcome {
            text,
            is_error: false,
        },
        Err(e) => fail(e.to_string()),
    }
}

/// The orchestrator: a new Work chat goes to the agent that fits it best, when that is turned on
/// and one clearly fits; the chat says so. Any failure leaves the chat as it was.
pub(super) async fn route_to_agent(host: &dyn Host, mut request: TurnRequest) -> TurnRequest {
    let Ok(dir) = host.config_dir() else {
        return request;
    };
    let fresh = store::load_conversation(&dir, &request.conversation_id).is_err();
    if !fresh
        || request.agent_id.is_some()
        || request.workdir.is_some()
        || request.text.trim().is_empty()
        || !agents::routing_enabled(&dir)
    {
        return request;
    }
    let team = agents::load(&dir);
    if team.is_empty() {
        return request;
    }
    // The fast model decides when Auto has one; otherwise the chat's own.
    let reference = crate::chat::router::models(&providers::load(&dir))
        .map_or_else(|| request.model.clone(), |(fast, _)| fast);
    let ask = CompletionRequest::new(Message::user(agents::route_prompt(&team, &request.text)));
    let reply = match ask_text(&dir, &reference, ask, 30).await {
        Ok(r) => r,
        Err(e) => {
            warn!("could not route the chat to an agent: {e}");
            return request;
        }
    };
    if let Some(agent) = agents::parse_route(&team, &reply) {
        info!("routed a new chat to {}", agent.name);
        request.agent_id = Some(agent.id.clone());
        let note = format!("Cortex handed this chat to {}.", agent.name);
        request.route_note = Some(match request.route_note.take() {
            Some(before) => format!("{before} {note}"),
            None => note,
        });
    }
    request
}

/// The Auto model: picks the fast or the strong model for this message and says which.
pub(super) fn auto_route(
    host: &dyn Host,
    mut request: TurnRequest,
) -> Result<TurnRequest, ChatError> {
    if request.model != crate::chat::router::AUTO {
        return Ok(request);
    }
    let dir = host.config_dir()?;
    let (fast, strong) = crate::chat::router::models(&providers::load(&dir)).ok_or_else(|| {
        ChatError::Invalid(
            "Choose Auto's fast and strong models under Settings → Providers & models first."
                .into(),
        )
    })?;
    // Regenerating: the message being answered again decides.
    let text = if request.text.trim().is_empty() {
        store::load_conversation(&dir, &request.conversation_id)
            .ok()
            .and_then(|c| store::last_prompt(&c).and_then(|i| c.messages.get(i).cloned()))
            .map(|m| match m {
                Message::User { content } => content
                    .iter()
                    .filter_map(|c| {
                        if let rig_core::completion::message::UserContent::Text(t) = c {
                            Some(t.text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            })
            .unwrap_or_default()
    } else {
        request.text.clone()
    };
    let route = crate::chat::router::route(&text, request.images.len());
    request.model = if route.strong { strong } else { fast };
    let name = providers::split_reference(&request.model)
        .map_or(request.model.as_str(), |(_, m)| m)
        .to_string();
    request.route_note = Some(format!("Auto chose {name}: {}.", route.reason));
    Ok(request)
}
