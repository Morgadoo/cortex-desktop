//! The state of a running turn and its steps for the kernel's loop: one model call, then the tools it called.

use super::*;

/// A turn's state across its steps, for the kernel's loop ([`cortex_kernel::agent_loop`]): each
/// step streams one model call, then runs the tools it called.
pub(super) struct TurnSteps<'a> {
    pub(super) host: &'a Arc<dyn Host>,
    pub(super) request: &'a TurnRequest,
    pub(super) cancel: watch::Receiver<bool>,
    pub(super) conversation: Conversation,
    pub(super) model: DynModel<Completion>,
    pub(super) model_ref: String,
    pub(super) fallback: Option<String>,
    pub(super) redactions: privacy::Redactions,
    pub(super) compaction_failed: bool,
    pub(super) system: String,
    pub(super) offered: Vec<ChatTool>,
    pub(super) defer: bool,
    pub(super) agent: Option<agents::Agent>,
    pub(super) config_dir: std::path::PathBuf,
    pub(super) id: &'a str,
    pub(super) window: u64,
    pub(super) filter: bool,
    pub(super) reasoning: bool,
    pub(super) run: &'a str,
    /// The tool calls of the step just made, for `run_calls`.
    pub(super) calls: Vec<ToolCall>,
}

#[async_trait::async_trait]
impl Steps for TurnSteps<'_> {
    async fn call_model(&mut self, step: usize, last: bool) -> Result<Reply, String> {
        let TurnSteps {
            host,
            request,
            cancel,
            conversation,
            model,
            model_ref,
            fallback,
            redactions,
            compaction_failed,
            system,
            offered,
            defer,
            agent,
            config_dir,
            id,
            window,
            filter,
            reasoning,
            run,
            calls: pending,
        } = self;
        let (host, request, id, run) = (*host, *request, *id, *run);
        let (defer, window, filter, reasoning) = (*defer, *window, *filter, *reasoning);
        let config_dir = config_dir.as_path();
        // Guidance the user sent while this turn was running joins the conversation here.
        for text in host.chat_state().take_steering(id) {
            conversation.messages.push(Message::user(format!(
                "(Sent while you were working) {text}"
            )));
            conversation
                .times
                .push((conversation.messages.len() - 1, Utc::now()));
            host.emit(ChatEvent::Steered {
                conversation_id: id.into(),
                text,
            });
        }
        if !*compaction_failed {
            *compaction_failed =
                compact_if_needed(host.as_ref(), model, conversation, window, config_dir).await
                    == Compaction::Failed;
        }
        let mut carried =
            context::request_messages(&conversation.messages, conversation.summary.as_ref());
        if filter {
            carried = privacy::redact_messages(&carried, redactions);
        }
        let Some((prompt, history)) = carried.split_last() else {
            return Err("empty conversation".into());
        };
        let last_step = last;
        let mut step_system = match &conversation.summary {
            Some(summary) => format!(
                "{system}\n\nSummary of the earlier part of this conversation (those messages are no longer shown):\n{}",
                summary.text
            ),
            None => system.clone(),
        };
        if filter {
            step_system = redactions.redact(&step_system);
        }
        if last_step {
            step_system.push_str("\n\nYou have used every tool step available for this request. Answer now from what you have found, and say what you could not finish.");
        }
        // An agent's reminder rides on the newest message the user typed, every request; it is
        // not saved with the conversation.
        let mut prompt = prompt.clone();
        if let Some(reminder) = agent
            .as_ref()
            .map(|a| a.reminder.trim())
            .filter(|r| !r.is_empty())
        {
            if let Message::User { content } = &mut prompt {
                if let Some(rig_core::completion::message::UserContent::Text(t)) = content
                    .iter_mut()
                    .find(|c| matches!(c, rig_core::completion::message::UserContent::Text(_)))
                {
                    t.text = format!("{}\n\n(Reminder: {reminder})", t.text);
                }
            }
        }
        let call = completion_request(
            prompt,
            history,
            step_system,
            last_step,
            offered,
            defer,
            conversation,
            config_dir,
            request,
            reasoning,
        );

        // Transient failures (dropped connections, rate limits, overloaded servers) are retried
        // with backoff, as long as nothing reached the user yet.
        let mut attempt = 0u32;
        let call_started = std::time::Instant::now();
        // When the model started and stopped thinking, for "Thought for 4s".
        let mut thought_ms: Option<u32>;
        let response = loop {
            attempt += 1;
            let mut stream = model.stream(call.clone()).map_err(|e| e.to_string())?;
            let read = read_stream(&mut stream, cancel, host, id, filter, redactions).await;
            let (was_cancelled, streamed_text, streamed_anything, thinking) =
                (read.cancelled, read.text, read.anything, read.thinking);

            if was_cancelled {
                if !streamed_text.trim().is_empty() {
                    conversation
                        .messages
                        .push(Message::assistant(streamed_text));
                    conversation
                        .times
                        .push((conversation.messages.len() - 1, Utc::now()));
                }
                conversation.updated_at = Utc::now();
                store::save_conversation(config_dir, conversation)?;
                return Ok(Reply::Cancelled);
            }

            thought_ms = thinking.map(|(start, end)| {
                end.unwrap_or_else(std::time::Instant::now)
                    .duration_since(start)
                    .as_millis()
                    .min(u32::MAX as u128) as u32
            });
            let finished = stream.finish().await;
            if let Err(e) = &finished {
                crate::chat::traces::model_call(
                    config_dir,
                    run,
                    step,
                    model_ref,
                    call_started.elapsed().as_millis() as u64,
                    None,
                    Some(if e.is_retryable() {
                        "transient"
                    } else {
                        "failed"
                    }),
                );
            }
            match finished {
                Ok(response) => {
                    crate::chat::traces::model_call(
                        config_dir,
                        run,
                        step,
                        model_ref,
                        call_started.elapsed().as_millis() as u64,
                        Some(&response.usage),
                        None,
                    );
                    break response;
                }
                Err(e) if e.is_retryable() && !streamed_anything && attempt < MAX_ATTEMPTS => {
                    let wait = std::time::Duration::from_secs(2u64.pow(attempt));
                    warn!("model call failed ({e}); retrying in {}s", wait.as_secs());
                    tokio::select! {
                        _ = cancelled(cancel) => return Ok(Reply::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
                Err(e) if !streamed_anything && fallback.is_some() => {
                    // Fail over to the agent's backup model, once, and say so.
                    let backup = fallback.take().unwrap_or_default();
                    warn!("model call failed ({e}); switching to {backup}");
                    match host.connect_model(&backup).await {
                        Ok(next) => {
                            *model = next;
                            model_ref.clone_from(&backup);
                            attempt = 0;
                            let label = backup.rsplit('/').next().unwrap_or(&backup).to_string();
                            let text = format!(
                                "{}; continuing with {label}.",
                                crate::chat::context::brief_error(&e.to_string())
                            );
                            conversation
                                .notices
                                .push((conversation.messages.len(), text.clone()));
                            host.emit(ChatEvent::Notice {
                                conversation_id: id.into(),
                                text,
                            });
                        }
                        Err(_) => {
                            store::save_conversation(config_dir, conversation)?;
                            return Err(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    store::save_conversation(config_dir, conversation)?;
                    return Err(e.to_string());
                }
            }
        };
        conversation.usage.add(&response.usage);
        host.emit(ChatEvent::Usage {
            conversation_id: id.into(),
            usage: conversation.usage.clone(),
            context_window: window,
        });
        let message = response.message().map(|m| {
            if filter {
                privacy::restore_message(m, redactions)
            } else {
                m
            }
        });
        let calls: Vec<ToolCall> = match &message {
            Some(Message::Assistant(a)) => a
                .content
                .iter()
                .filter_map(|c| match c {
                    rig_core::completion::message::AssistantContent::ToolCall(call) => {
                        Some(call.clone())
                    }
                    _ => None,
                })
                .collect(),
            _ => response.tool_calls().cloned().collect(),
        };
        if let Some(message) = message {
            conversation.messages.push(message);
            if let Some(ms) = thought_ms {
                conversation
                    .thought
                    .push((conversation.messages.len() - 1, ms));
            }
        }
        conversation.updated_at = Utc::now();

        if calls.is_empty() {
            conversation
                .times
                .push((conversation.messages.len() - 1, Utc::now()));
            store::save_conversation(config_dir, conversation)?;
            return Ok(Reply::Answered);
        }
        store::save_conversation(config_dir, conversation)?;
        info!("chat step {step}: {} tool call(s)", calls.len());
        *pending = calls;
        Ok(Reply::CalledTools)
    }

    async fn run_calls(&mut self, _step: usize) -> Result<bool, String> {
        let TurnSteps {
            host,
            request,
            cancel,
            conversation,
            model,
            offered,
            agent,
            config_dir,
            id,
            run,
            calls: pending,
            ..
        } = self;
        let (host, request, id, run) = (*host, *request, *id, *run);
        let config_dir = config_dir.as_path();
        let calls = std::mem::take(pending);
        for call in &calls {
            host.emit(ChatEvent::ToolStart {
                conversation_id: id.into(),
                call_id: call.id.to_string(),
                name: call.function.name.to_string(),
                arguments: Value::Object(call.function.arguments.clone()),
            });
        }

        // Decided once: an approval given during the step ("Always allow") must not move a call
        // out of the sequential queue after it was kept out of the concurrent batch.
        let sequential: Vec<bool> = calls
            .iter()
            .map(|call| waits_on_user(call, host, id, agent.as_ref(), offered, request))
            .collect();
        let mut outcomes: Vec<Option<tools::Outcome>> = (0..calls.len()).map(|_| None).collect();
        let runner = Runner {
            host,
            run,
            config_dir,
            offered: offered.as_slice(),
            model: &*model,
            request,
            id,
            agent: agent.as_ref(),
            workdir: conversation.workdir.clone(),
        };
        let concurrent = futures::future::join_all(
            calls
                .iter()
                .enumerate()
                .filter(|(i, _)| !sequential[*i])
                .map(|(i, c)| {
                    let runner = &runner;
                    async move { (i, runner.run(c).await) }
                }),
        );
        let finished = tokio::select! {
            _ = cancelled(cancel) => None,
            done = concurrent => Some(done),
        };
        if let Some(done) = finished {
            for (i, outcome) in done {
                outcomes[i] = Some(outcome);
            }
        }
        for (i, call) in calls.iter().enumerate() {
            if outcomes[i].is_some() || !sequential[i] || *cancel.borrow() {
                continue;
            }
            let outcome = if call.function.name.as_str() == ASK_USER
                && request.approval.unattended()
            {
                tools::Outcome::ok("Nobody is available to answer during this scheduled run; continue with your best judgement and state your assumptions.")
            } else if call.function.name.as_str() == ASK_USER {
                ask_user(host.as_ref(), id, call, cancel).await
            } else {
                tokio::select! {
                    _ = cancelled(cancel) => cancelled_outcome(),
                    outcome = runner.run(call) => outcome,
                }
            };
            outcomes[i] = Some(outcome);
        }

        let mut results = Vec::with_capacity(calls.len());
        for (call, outcome) in calls.iter().zip(outcomes) {
            let outcome = outcome.unwrap_or_else(cancelled_outcome);
            host.emit(ChatEvent::ToolEnd {
                conversation_id: id.into(),
                call_id: call.id.to_string(),
                output: outcome.text.clone(),
                is_error: outcome.is_error,
            });
            track_loaded_tools(conversation, call, &outcome);
            let content = result_content(config_dir, id, conversation, call, outcome.text);
            let mut result = call.result(content);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        conversation.messages.push(Message::tool_results(results));
        conversation.updated_at = Utc::now();
        store::save_conversation(config_dir, conversation)?;

        Ok(*cancel.borrow())
    }

    fn has_new_guidance(&self) -> bool {
        self.host.chat_state().has_steering(self.id)
    }
}

/// Calls that wait on the user run one at a time, after the rest have run at once: questions,
/// tools the agent asks about first, and connector calls that need approval.
fn waits_on_user(
    call: &ToolCall,
    host: &Arc<dyn Host>,
    id: &str,
    agent: Option<&agents::Agent>,
    offered: &[ChatTool],
    request: &TurnRequest,
) -> bool {
    let name = call.function.name.as_str();
    name == ASK_USER
        || (agent.is_some_and(|a| agents::asks_first(a, name))
            && !host.chat_state().session_allowed(id, name))
        || offered.iter().find(|t| t.name == name).is_some_and(|tool| {
            tool.connector_id.is_some()
                && tools::needs_approval(
                    tool,
                    &Value::Object(call.function.arguments.clone()),
                    agents::mode_for(agent, name, request.approval),
                )
        })
}

/// What `find_tools` found joins the conversation's loaded tools; what `unload_tools` names
/// leaves them.
fn track_loaded_tools(
    conversation: &mut store::Conversation,
    call: &ToolCall,
    outcome: &tools::Outcome,
) {
    if call.function.name.as_str() == deferred::FIND_TOOLS && !outcome.is_error {
        // What it found is loaded for the next step and the rest of the conversation.
        if let Ok(found) = serde_json::from_str::<Value>(&outcome.text) {
            for name in found["matches"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["name"].as_str())
            {
                if !conversation.loaded_tools.iter().any(|t| t == name) {
                    conversation.loaded_tools.push(name.to_string());
                }
            }
        }
    }
    if call.function.name.as_str() == handlers::UNLOAD_TOOLS && !outcome.is_error {
        // Unloaded tools leave the next request; find_tools brings them back.
        if let Ok(gone) = serde_json::from_str::<Value>(&outcome.text) {
            let gone: Vec<&str> = gone["unloaded"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            conversation
                .loaded_tools
                .retain(|t| !gone.contains(&t.as_str()));
        }
    }
}

/// A tool result's content: its text, cut to its start and end when long (the whole kept in a
/// file), and for computer use, the screenshot taken with it.
fn result_content(
    config_dir: &std::path::Path,
    id: &str,
    conversation: &mut store::Conversation,
    call: &ToolCall,
    mut text: String,
) -> Vec<ToolResultContent> {
    // A long output is kept whole in a file; the conversation keeps its start and end,
    // which is what the model reads, with how to read the rest.
    if text.chars().count() > context::MAX_RESULT_CHARS {
        match save_tool_output(config_dir, id, &call.id.to_string(), &text) {
            Ok(()) => text = context::head_and_tail(&text, &call.id.to_string()),
            Err(e) => warn!("could not keep a long tool output: {e}"),
        }
    }
    let mut content = vec![ToolResultContent::text(text)];
    // Computer use: the screenshot goes with the result; older ones leave the history.
    if call.function.name.as_str() == crate::chat::computer::TOOL {
        if let Some(data) = crate::chat::computer::take_image(id) {
            drop_old_screenshots(&mut conversation.messages);
            content.push(ToolResultContent::image_base64(
                data,
                Some(rig_core::completion::message::ImageMediaType::JPEG),
                None,
            ));
        }
    }
    content
}

/// Restores placeholders in streamed text. A placeholder can arrive split across chunks, so an
/// unclosed `[` is held back until its `]` arrives (or it is clearly not a placeholder).
pub(super) fn restore_streamed(
    redactions: &privacy::Redactions,
    held: &mut String,
    chunk: &str,
) -> String {
    held.push_str(chunk);
    let cut = match held.rfind('[') {
        Some(open) if !held[open..].contains(']') && held.len() - open < 24 => open,
        _ => held.len(),
    };
    let ready: String = held.drain(..cut).collect();
    redactions.restore(&ready)
}
