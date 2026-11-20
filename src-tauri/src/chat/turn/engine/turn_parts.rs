//! The parts of a turn: putting the request into the conversation, the back ends that answer in a model's place, the tools offered, the prompt's per-turn sections, one step's request, and reading its streamed reply.

use super::*;
use crate::chat::error::ChatError;

/// Titles a new conversation in the background, once its first answer is in.
pub(super) fn name_unless(
    named: bool,
    host: &Arc<dyn Host>,
    request: &TurnRequest,
    conversation: Conversation,
) {
    if !named {
        let host = host.clone();
        let reference = request.model.clone();
        tauri::async_runtime::spawn(async move {
            name_conversation(host, reference, conversation).await
        });
    }
}

/// A pipe (a function of the user's) answers with whatever its code prints. Returns whether the
/// user cancelled.
pub(super) async fn answer_with_pipe(
    host: &Arc<dyn Host>,
    request: &TurnRequest,
    config_dir: &std::path::Path,
    mut conversation: Conversation,
    cancel: &mut watch::Receiver<bool>,
    pipe: &crate::chat::functions::Function,
) -> Result<bool, ChatError> {
    let id = request.conversation_id.as_str();
    let named = !conversation.title.is_empty();
    let args = serde_json::json!({ "messages": crate::chat::functions::messages(&conversation), "model": pipe.id });
    let answer = tokio::select! {
        out = crate::chat::functions::run(config_dir, id, pipe, &args) => Some(out?),
        _ = cancelled(cancel) => None,
    };
    let Some(answer) = answer else {
        return Ok(true);
    };
    if answer.trim().is_empty() {
        return Err(ChatError::Invalid(format!(
            "{} printed nothing.",
            pipe.name
        )));
    }
    host.emit(ChatEvent::Text {
        conversation_id: id.into(),
        text: answer.clone(),
    });
    conversation.messages.push(Message::assistant(answer));
    conversation
        .times
        .push((conversation.messages.len() - 1, Utc::now()));
    conversation.updated_at = Utc::now();
    store::save_conversation(config_dir, &conversation)?;
    name_unless(named, host, request, conversation);
    Ok(false)
}

/// The prompt's sections that depend on this turn rather than on who the user is: secrets the
/// run may use, this chat's own instructions, tools to load, the agent's notebook and team,
/// knowledge pages for this message, and plan mode.
#[allow(clippy::too_many_arguments)]
pub(super) fn add_turn_sections(
    prompt: &mut crate::chat::prompt::Prompt,
    config_dir: &std::path::Path,
    request: &TurnRequest,
    conversation: &Conversation,
    agent: Option<&agents::Agent>,
    offered: &[ChatTool],
    defer: bool,
    memories: &memory::Memories,
    pages: &[knowledge::Page],
    latest_prompt: &str,
) {
    use crate::chat::prompt::{ESSENTIAL, OPTIONAL, USEFUL};
    if let Some(secrets) = crate::chat::vault::prompt(&crate::chat::vault::granted(
        &crate::chat::vault::list(config_dir),
        agent.map(|a| a.secrets.as_slice()),
    )) {
        prompt.push("vault", ESSENTIAL, secrets);
    }
    if !conversation.params.instructions.is_empty() {
        prompt.push(
            "chat instructions",
            ESSENTIAL,
            format!(
                "\n\n## Instructions for this conversation\n{}",
                conversation.params.instructions
            ),
        );
    }
    if defer {
        prompt.push(
            "tools to load",
            ESSENTIAL,
            deferred::prompt(offered, &conversation.loaded_tools),
        );
    }
    if let Some(agent) = agent {
        prompt.push(
            "notebook",
            ESSENTIAL,
            agents::notebook_prompt(&agents::notebook(config_dir, &agent.id)),
        );
        if agent.can_manage_agents {
            prompt.push(
                "organization",
                USEFUL,
                crate::chat::org::prompt(&agents::load(config_dir), agent),
            );
        }
    }
    if offered.iter().any(|t| t.name == builtins::DELEGATE) {
        let all = agents::load(config_dir);
        // A group hosts its members; any other chat may call on the whole team.
        match agent.and_then(|a| agents::group_prompt(&all, a)) {
            Some(group) => prompt.push("group", USEFUL, group),
            None => {
                if let Some(team) = agents::team_prompt(&all, agent.map(|a| a.id.as_str())) {
                    prompt.push("team", OPTIONAL, team);
                }
            }
        }
    }
    // Knowledge pages that matter for this message come in full; the rest by title.
    if memories.enabled {
        if let Some(section) = knowledge::prompt(pages, latest_prompt) {
            prompt.push("knowledge pages", OPTIONAL, section);
        }
    }
    if request.approval == ApprovalMode::Plan {
        prompt.push(
            "plan mode",
            ESSENTIAL,
            "\n\nPlan mode is on. Investigate with read-only tools, then present a numbered plan \
             of the changes you would make (files, commands, risks) and stop. Do not try to change \
             anything; the user approves the plan and then switches to an editing mode.",
        );
    }
}

/// Puts the request into the conversation: an edited prompt (the old one kept as a branch), a
/// nudge to continue, a rewind to regenerate, or the new message.
pub(super) fn apply_request(
    conversation: &mut Conversation,
    request: &TurnRequest,
    text: &str,
) -> Result<(), ChatError> {
    if let Some(index) = request.edit_index {
        let is_prompt = conversation.messages.get(index).is_some_and(|m| {
            matches!(m, Message::User { content } if content.iter().any(|c| matches!(c, rig_core::completion::message::UserContent::Text(_))))
        });
        if !is_prompt || text.is_empty() {
            return Err(ChatError::Invalid("That message cannot be edited.".into()));
        }
        // The old message and everything after it stay available as a branch.
        store::keep_branch(conversation, index, "prompt");
        conversation.messages.truncate(index);
        conversation.times.retain(|(i, _)| *i < index);
        conversation.ratings.retain(|(i, _)| *i < index);
        if conversation
            .summary
            .as_ref()
            .is_some_and(|s| s.upto > index)
        {
            conversation.summary = None;
        }
        conversation.messages.push(Message::user(text));
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
    } else if request.resume {
        if conversation.messages.is_empty() {
            return Err(ChatError::Invalid("There is nothing to continue.".into()));
        }
        // A reply that ended mid-way may end on an assistant message; nudge it to go on.
        if matches!(conversation.messages.last(), Some(Message::Assistant(_))) {
            conversation.messages.push(Message::user("Continue."));
        }
    } else if request.text.trim().is_empty() {
        if !rewind_to_last_prompt(conversation) {
            return Err(ChatError::Invalid(
                "There is nothing to regenerate yet.".into(),
            ));
        }
    } else {
        conversation
            .messages
            .push(user_message(text, &request.images)?);
        conversation
            .times
            .push((conversation.messages.len() - 1, Utc::now()));
    }
    Ok(())
}

/// The request for one step: the conversation so far, the step's system prompt, the tools on
/// offer (none on the last step; deferred ones once loaded), and the chat's model settings.
#[allow(clippy::too_many_arguments)]
pub(super) fn completion_request(
    prompt: Message,
    history: &[Message],
    step_system: String,
    last_step: bool,
    offered: &[ChatTool],
    defer: bool,
    conversation: &Conversation,
    config_dir: &std::path::Path,
    request: &TurnRequest,
    reasoning: bool,
) -> CompletionRequest {
    let mut call = CompletionRequest::new(prompt)
        .messages(history.iter().cloned())
        .preamble(step_system);
    if !last_step {
        // Deferred tools go out once they are loaded; the rest every time.
        let definitions = offered
            .iter()
            .filter(|t| {
                !defer || !deferred::deferrable(t) || conversation.loaded_tools.contains(&t.name)
            })
            .filter_map(ChatTool::definition);
        call = call.tools(definitions);
    }
    // This conversation's own model settings, then the app-wide defaults.
    let params = conversation
        .params
        .or_defaults(&store::chat_defaults(config_dir));
    if let Some(t) = params.temperature {
        call = call.temperature(t);
    }
    if let Some(n) = params.max_tokens {
        call = call.max_tokens(n);
    }
    if let Some(p) = params.top_p {
        call = call.top_p(p);
    }
    if let Some(effort) = request.speed.effort().filter(|_| reasoning) {
        call = call.reasoning(effort);
    }
    call
}

/// What reading one streamed reply produced.
pub(super) struct StreamRead {
    /// The user stopped it.
    pub(super) cancelled: bool,
    /// The answer's text as the user saw it (personal data restored).
    pub(super) text: String,
    /// Whether anything reached the user, after which a failure is not retried.
    pub(super) anything: bool,
    /// When the model started and stopped thinking.
    pub(super) thinking: Option<(std::time::Instant, Option<std::time::Instant>)>,
}

/// Reads a streamed reply to its end, or until the user stops it, showing its text, reasoning
/// and pending tool calls as they come.
pub(super) async fn read_stream(
    stream: &mut rig_core::streaming::Streamed<Completion>,
    cancel: &mut watch::Receiver<bool>,
    host: &Arc<dyn Host>,
    id: &str,
    filter: bool,
    redactions: &privacy::Redactions,
) -> StreamRead {
    let mut thinking: Option<(std::time::Instant, Option<std::time::Instant>)> = None;
    let mut streamed_text = String::new();
    let mut streamed_anything = false;
    let mut held = String::new();
    let cancelled_by_user = loop {
        tokio::select! {
            _ = cancelled(cancel) => break true,
            item = stream.next() => match item {
                None => break false,
                Some(Err(_)) => break false, // finish() reports it
                Some(Ok(Item::Unknown(_))) => {}
                Some(Ok(Item::Event(event))) => match event {
                    StreamEvent::Text { text, .. } => {
                        streamed_anything = true;
                        if let Some((_, end @ None)) = &mut thinking {
                            *end = Some(std::time::Instant::now());
                        }
                        let text = if filter { restore_streamed(redactions, &mut held, &text) } else { text };
                        streamed_text.push_str(&text);
                        if !text.is_empty() {
                            host.emit(ChatEvent::Text { conversation_id: id.into(), text });
                        }
                    }
                    StreamEvent::Reasoning { text, .. } => {
                        streamed_anything = true;
                        thinking.get_or_insert((std::time::Instant::now(), None));
                        host.emit(ChatEvent::Reasoning { conversation_id: id.into(), text });
                    }
                    StreamEvent::Start { kind: PartKind::ToolCall, name: Some(name), .. } => {
                        streamed_anything = true;
                        if let Some((_, end @ None)) = &mut thinking {
                            *end = Some(std::time::Instant::now());
                        }
                        host.emit(ChatEvent::ToolPending { conversation_id: id.into(), name: name.to_string() });
                    }
                    _ => {}
                },
            },
        }
    };
    StreamRead {
        cancelled: cancelled_by_user,
        text: streamed_text,
        anything: streamed_anything,
        thinking,
    }
}

/// The tools a turn offers the model: the connectors', the built-ins and the user's own, less
/// what the agent, the chat or the run rules out; and whether some load only on demand.
pub(super) fn tools_for_turn(
    registry: &crate::tools::registry::ToolRegistry,
    config_dir: &std::path::Path,
    request: &TurnRequest,
    conversation: &Conversation,
    agent: Option<&agents::Agent>,
) -> (Vec<ChatTool>, bool) {
    let mut offered = tools::available(registry, request.connectors.as_deref());
    offered.splice(1..1, builtins::definitions(config_dir));
    offered.push(read_tool_output_tool());
    offered.extend(crate::chat::openapi::tools(config_dir));
    offered.extend(crate::chat::custom_tools::definitions(config_dir));
    // Tools the assistant wrote and the user approved, and the way to write more.
    offered.extend(crate::chat::script_tools::definitions(config_dir));
    offered.extend(crate::chat::script_tools::builtin_definitions());
    if !conversation.temporary {
        offered.push(crate::chat::proposals::definition());
    }
    if crate::chat::computer::load(config_dir).enabled {
        offered.push(crate::chat::computer::definition());
    }
    if let Some(agent) = agent {
        offered.retain(|t| t.name == ASK_USER || agents::allows(agent, &t.name));
    }
    // The composer's Tools menu: what the user turned off for this chat.
    if !request.tools_off.is_empty() {
        offered.retain(|t| {
            !request
                .tools_off
                .iter()
                .any(|off| off == &t.name || t.name.starts_with(&format!("{off}__")))
        });
    }
    if let Some(only) = &request.only_tools {
        offered.retain(|t| only.contains(&t.name));
    }
    if request.approval.unattended() {
        // Nobody can answer a question during a scheduled run.
        offered.retain(|t| t.name != ASK_USER);
    }
    if agent.is_some() {
        // An agent keeps a notebook across its conversations and runs.
        offered.push(agents::notebook_tool());
    }
    if agent.is_some_and(|a| a.can_manage_agents) {
        // A manager, as the user allowed, builds and runs its own organization of agents.
        offered.push(crate::chat::org::definition());
    }
    if conversation.workdir.is_some() {
        // Coding sessions get a terminal for long-running commands.
        offered.extend(terminal::definitions());
    }
    if conversation.temporary {
        // A temporary chat leaves no trace: it does not add to or change knowledge.
        // Nor the assistant's own tools and skills: an eval case or a worker runs in one.
        offered.retain(|t| {
            !matches!(
                t.name.as_str(),
                builtins::REMEMBER
                    | builtins::FORGET
                    | builtins::KNOWLEDGE_WRITE
                    | builtins::SAVE_SKILL
                    | crate::chat::script_tools::CREATE_TOOL
                    | crate::chat::script_tools::PROMOTE_TOOL
                    | crate::chat::script_tools::RETIRE_TOOL
                    | crate::chat::script_tools::REQUEST_CAPABILITY
            )
        });
    }
    // Past a few dozen connector tools, the rest load on demand (Vibe's "Found tools").
    let defer = deferred::should_defer(&offered);
    if defer {
        offered.push(deferred::find_tools_tool());
        offered.push(handlers::unload_tools_tool());
    }
    offered.push(handlers::context_status_tool());
    (offered, defer)
}
