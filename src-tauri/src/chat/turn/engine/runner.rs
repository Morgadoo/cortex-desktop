//! Running one tool call: policy, approval, hooks, the call itself and what is recorded.

use super::*;

/// Runs one tool call, `run`, the way every loop that calls tools must: between the user's
/// `before_tool` and `after_tool` hooks, with `agent`'s vault secrets reaching the processes it
/// starts and masked in what comes back, and recorded in the activity log.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn guarded_call<F>(
    config_dir: &std::path::Path,
    conversation_id: &str,
    trace: Option<&str>,
    agent: Option<&agents::Agent>,
    workdir: Option<&std::path::Path>,
    name: &str,
    arguments: &Value,
    run: F,
) -> tools::Outcome
where
    F: std::future::Future<Output = tools::Outcome>,
{
    if let Some(reason) =
        crate::chat::hooks::before_tool(config_dir, conversation_id, name, arguments, workdir).await
    {
        return tools::Outcome::err(format!("A hook blocked this call: {reason}"));
    }
    // The agent's vault secrets reach the processes this call starts, and never the model.
    let secrets =
        crate::chat::vault::env_for(config_dir, agent.map(|a| a.secrets.as_slice())).await;
    let started = std::time::Instant::now();
    // A written tool records its runs itself, in this run's trace.
    let run = crate::chat::script_tools::in_run(trace.unwrap_or_default().to_string(), run);
    let mut outcome = crate::chat::vault::scope(secrets.clone(), run).await;
    if !secrets.is_empty() {
        outcome.text = crate::chat::vault::mask(&outcome.text, &secrets);
    }
    let shown_args = if secrets.is_empty() {
        arguments.to_string()
    } else {
        crate::chat::vault::mask(&arguments.to_string(), &secrets)
    };
    crate::chat::audit::record(
        config_dir,
        crate::chat::audit::Entry {
            at: Utc::now(),
            kind: "tool".into(),
            conversation: conversation_id.to_string(),
            agent: agent.map(|a| a.name.clone()),
            tool: name.to_string(),
            arguments: crate::chat::audit::short(&shown_args),
            outcome: if outcome.is_error { "error" } else { "ok" }.into(),
            detail: crate::chat::audit::short(&outcome.text),
            millis: started.elapsed().as_millis() as u64,
        },
    );
    if let Some(trace) = trace {
        if name.starts_with("script__") {
            // A written tool records its own runs where it actually ran (also when another tool
            // calls it), so a refusal before it runs (declined, nobody to approve, a changed or
            // unapproved tool, the rate limit) is not counted against it as a failure. A run that
            // fails, a lapsed grant's denial included, is.
            if !outcome.is_error {
                crate::chat::traces::attribute(config_dir, trace, "tool", name, true);
            }
        } else {
            crate::chat::traces::tool_call(
                config_dir,
                trace,
                name,
                started.elapsed().as_millis() as u64,
                !outcome.is_error,
            );
        }
    }
    if !outcome.is_error && crate::chat::traces::empty_result(&outcome.text) {
        crate::chat::traces::signal(config_dir, conversation_id, None, "empty_result");
    }
    let notes = crate::chat::hooks::after_tool(
        config_dir,
        conversation_id,
        name,
        arguments,
        &outcome.text,
        outcome.is_error,
        workdir,
    )
    .await;
    if !notes.is_empty() {
        outcome.text.push_str(&format!(
            "\n\n[From the user's hooks]\n{}",
            notes.join("\n")
        ));
    }
    outcome
}

/// What one tool call needs to run, shared by the calls of a step.
pub(super) struct Runner<'a> {
    pub(super) host: &'a Arc<dyn Host>,
    /// The trace this turn records into.
    pub(super) run: &'a str,
    pub(super) config_dir: &'a std::path::Path,
    pub(super) offered: &'a [ChatTool],
    pub(super) model: &'a DynModel<Completion>,
    pub(super) request: &'a TurnRequest,
    pub(super) id: &'a str,
    pub(super) agent: Option<&'a agents::Agent>,
    /// The chat's folder, where hooks run.
    pub(super) workdir: Option<String>,
}

impl Runner<'_> {
    /// Runs a call between the user's `before_tool` and `after_tool` hooks.
    pub(super) async fn run(&self, call: &ToolCall) -> tools::Outcome {
        let arguments = Value::Object(call.function.arguments.clone());
        guarded_call(
            self.config_dir,
            self.id,
            Some(self.run),
            self.agent,
            self.workdir.as_deref().map(std::path::Path::new),
            call.function.name.as_str(),
            &arguments,
            self.run_inner(call),
        )
        .await
    }

    /// A change made outside the connectors (the organization, a scheduled task, a saved skill)
    /// under the approval mode; see [`change_gate`]. `Err` carries the refusal for the model.
    /// `connector` labels the approval card; `None` shows the tool on its own.
    pub(super) async fn confirm_change(
        &self,
        name: &str,
        connector: Option<String>,
        arguments: &Value,
        asked: bool,
        in_plan: &str,
        unattended: &str,
    ) -> Result<(), tools::Outcome> {
        let refuse = |text: &str| Err(tools::Outcome::err(text));
        match change_gate(
            self.request.approval,
            asked || self.host.chat_state().session_allowed(self.id, name),
        ) {
            ChangeGate::Run => Ok(()),
            ChangeGate::RefuseInPlan => refuse(in_plan),
            ChangeGate::RefuseUnattended => refuse(unattended),
            ChangeGate::Ask => {
                let gate = ChatTool {
                    name: name.into(),
                    description: String::new(),
                    parameters: Value::Null,
                    connector_id: connector,
                    tool_name: name.into(),
                    read_only: false,
                };
                match self.host.approve(&gate, arguments, None).await {
                    ApprovalDecision::Deny => refuse(
                        "The user declined this change. Do not retry it; ask what they would like instead.",
                    ),
                    // "Always allow" lasts for this conversation, as in Vibe.
                    ApprovalDecision::Always => {
                        self.host.chat_state().allow_for_session(self.id, name);
                        Ok(())
                    }
                    ApprovalDecision::Once => Ok(()),
                }
            }
        }
    }

    /// `manage_agents`: a manager agent changes its organization, under the approval mode (it
    /// asks in Ask, is refused in Plan and in scheduled runs nobody watches), then the page's
    /// agent list refreshes.
    pub(super) async fn manage_agents(&self, arguments: &Value, asked: bool) -> tools::Outcome {
        let refuse = |text: &str| tools::Outcome::err(text);
        let Some(agent) = self.agent else {
            return refuse("Only agents can manage agents.");
        };
        if crate::chat::org::changes(arguments) {
            if let Err(refused) = self
                .confirm_change(
                    crate::chat::org::MANAGE_AGENTS,
                    Some(agent.name.clone()),
                    arguments,
                    asked,
                    "Plan mode changes nothing: describe the agents you would create or change, and the user can switch modes to go ahead.",
                    "Nobody is watching this scheduled run to approve changes to your organization; make them in a chat.",
                )
                .await
            {
                return refused;
            }
        }
        match crate::chat::org::apply(self.config_dir, &agent.id, arguments) {
            Ok((text, changed)) => {
                if changed {
                    if let Some(app) = self.host.app() {
                        let _ = app.emit("chat://agents-changed", ());
                    }
                }
                tools::Outcome {
                    text,
                    is_error: false,
                }
            }
            Err(e) => refuse(&e.to_string()),
        }
    }

    pub(super) async fn run_inner(&self, call: &ToolCall) -> tools::Outcome {
        let name = call.function.name.as_str();
        let arguments = Value::Object(call.function.arguments.clone());
        if let Some(raw) = &call.function.invalid_arguments {
            return tools::Outcome::err(format!("The arguments were not a JSON object: {raw}"));
        }
        // Only what this turn offered runs: an agent's allowed tools, a worker's or eval's
        // `only_tools`, the composer's tools turned off, a temporary chat's missing memory tools.
        // A model that names anything else, from an injected page or a guess, is refused.
        if !self.offered.iter().any(|t| t.name == name) {
            return tools::Outcome::err(format!("There is no tool named {name} here."));
        }
        // The agent's "ask first" policy covers every tool, built-in and reading ones included,
        // except those that always ask with their own fuller card (review, scores, access): asking
        // first would replace that card with a plain one.
        let mut asked = false;
        let asks_with_its_own_card = matches!(
            name,
            builtins::SAVE_SKILL
                | crate::chat::script_tools::PROMOTE_TOOL
                | crate::chat::script_tools::REQUEST_CAPABILITY
                | crate::chat::proposals::PROPOSE_CHANGE
        );
        if !asks_with_its_own_card
            && self.agent.is_some_and(|a| agents::asks_first(a, name))
            && !self.host.chat_state().session_allowed(self.id, name)
        {
            if self.request.approval.unattended() {
                return tools::Outcome { text: format!("{name} asks first under this agent's policy, and nobody is watching this scheduled run to approve it."), is_error: true };
            }
            let gate = ChatTool {
                name: name.into(),
                description: String::new(),
                parameters: Value::Null,
                connector_id: Some(
                    self.agent
                        .map_or_else(|| "agent".to_string(), |a| a.name.clone()),
                ),
                tool_name: name.into(),
                read_only: false,
            };
            match self.host.approve(&gate, &arguments, None).await {
                ApprovalDecision::Deny => {
                    return tools::Outcome::err("The user declined this tool call. Do not retry it; ask what they would like instead.");
                }
                // "Always allow" lasts for this conversation, as in Vibe.
                ApprovalDecision::Always => self.host.chat_state().allow_for_session(self.id, name),
                ApprovalDecision::Once => {}
            }
            asked = true;
        }
        if let Some(handler) = handlers::find(name) {
            return handler(self, call, arguments, asked).await;
        }
        match self.offered.iter().find(|t| t.name == name) {
            // Approved just now under the agent's policy: do not ask a second time.
            Some(tool) if asked => {
                tools::execute(
                    self.host.as_ref(),
                    tool,
                    arguments,
                    ApprovalMode::Auto,
                    self.id,
                )
                .await
            }
            Some(tool) => {
                let mode = agents::mode_for(self.agent, name, self.request.approval);
                // Auto: what Ask mode would confirm gets a quick check against the request.
                if mode == ApprovalMode::Auto
                    && tools::risk_here(Some(self.config_dir), tool, &arguments).is_none()
                    && tools::needs_approval(tool, &arguments, ApprovalMode::Ask)
                {
                    if let Some(reason) = self.auto_check(name, &arguments).await {
                        if self.host.approve(tool, &arguments, Some(&reason)).await
                            == ApprovalDecision::Deny
                        {
                            return tools::Outcome::err("The user declined this tool call. Do not retry it; ask what they would like instead.");
                        }
                    }
                }
                tools::execute(self.host.as_ref(), tool, arguments, mode, self.id).await
            }
            None => tools::Outcome::err(format!("There is no tool named {name}.")),
        }
    }
}

impl Runner<'_> {
    /// Background commands in the session's folder; starting one asks like any shell command.
    pub(super) async fn terminal(&self, name: &str, arguments: &Value) -> tools::Outcome {
        let fail = |text: &str| tools::Outcome::err(text);
        let arg = |k: &str| {
            arguments
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        match name {
            terminal::READ => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => {
                    let chars = arguments
                        .get("chars")
                        .and_then(Value::as_u64)
                        .unwrap_or(4000)
                        .clamp(200, 20_000) as usize;
                    tools::Outcome {
                        text: format!(
                            "Status: {}{}\n\n{}",
                            view.status,
                            view.exit_code
                                .map(|c| format!(" (exit {c})"))
                                .unwrap_or_default(),
                            terminal::tail(&view, chars)
                        ),
                        is_error: false,
                    }
                }
                _ => fail("There is no background command with that id here."),
            },
            terminal::STOP => match terminal::get(&arg("id")) {
                Some(view) if view.conversation_id == self.id => match terminal::stop(&view.id) {
                    Ok(()) => tools::Outcome::ok("Stopped."),
                    Err(e) => fail(&e.to_string()),
                },
                _ => fail("There is no background command with that id here."),
            },
            _ => {
                let Some(dir) = store::load_conversation(self.config_dir, self.id)
                    .ok()
                    .and_then(|c| c.workdir)
                else {
                    return fail("Background commands need a coding session with a folder.");
                };
                let gate = ChatTool {
                    name: "terminal__start".into(),
                    description: String::new(),
                    parameters: Value::Null,
                    connector_id: Some("terminal".into()),
                    tool_name: "start".into(),
                    read_only: false,
                };
                // Smart-approve: a command that is hard to undo, or that names the app's own
                // data, asks even in Full auto.
                let risky = tools::risk_here(Some(self.config_dir), &gate, arguments);
                match agents::mode_for(self.agent, terminal::START, self.request.approval) {
                    ApprovalMode::Plan => return fail("Plan mode: nothing is started yet. Put this command in your plan."),
                    ApprovalMode::Unattended => return fail("This scheduled run may not start commands."),
                    ApprovalMode::Autonomous if risky.is_some() => {
                        return fail(&format!("This command was not started: {}, and nobody is watching this scheduled run to approve it.", risky.unwrap_or_default()))
                    }
                    ApprovalMode::Autonomous if risky.is_none() => {}
                    // Auto: the harness evaluates the command against the request.
                    ApprovalMode::Auto if risky.is_none() => {
                        if let Some(reason) = self.auto_check(terminal::START, arguments).await {
                            if self.host.approve(&gate, arguments, Some(&reason)).await == ApprovalDecision::Deny {
                                return fail("The user declined to start this command. Do not retry it; ask what they would like instead.");
                            }
                        }
                    }
                    _ => {
                        if self.host.approve(&gate, arguments, None).await == ApprovalDecision::Deny {
                            return fail("The user declined to start this command. Do not retry it; ask what they would like instead.");
                        }
                    }
                }
                match terminal::start(self.id, &dir, &arg("command")) {
                    Ok(id) => {
                        // A moment for the first lines (a server's address, an early error).
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        let view = terminal::get(&id);
                        tools::Outcome {
                            text: format!(
                                "Started in the background with id {id}. Status: {}.\n{}",
                                view.as_ref().map_or("running", |v| v.status.as_str()),
                                view.as_ref()
                                    .map(|v| terminal::tail(v, 2000))
                                    .unwrap_or_default()
                            ),
                            is_error: false,
                        }
                    }
                    Err(e) => fail(&e.to_string()),
                }
            }
        }
    }

    /// Auto mode's judgement on one change: `None` to go ahead, or why to ask the user.
    pub(super) async fn auto_check(&self, tool: &str, arguments: &Value) -> Option<String> {
        let conversation = store::load_conversation(self.config_dir, self.id).ok()?;
        let request = store::last_prompt(&conversation)
            .and_then(|i| conversation.messages.get(i))
            .map(crate::chat::context::message_text)
            .unwrap_or_default();
        let prompt = tools::auto_check_prompt(&request, tool, arguments);
        let call = rig_core::completion::CompletionRequest::new(Message::user(prompt));
        match self.model.call(call).await {
            Ok(response) => tools::auto_check_verdict(&response.text()),
            // No answer: asking is the safe side.
            Err(e) => Some(format!(
                "the safety check could not run ({})",
                crate::chat::context::short_error(&e.to_string())
            )),
        }
    }

    /// Runs code once the user allows it: every run in "ask" modes, unless they allowed all, and
    /// every run in every mode where it would not be isolated.
    pub(super) async fn run_code(&self, arguments: &Value) -> tools::Outcome {
        let gate = sandbox::code_gate(
            self.request.approval,
            sandbox::isolated(self.config_dir),
            sandbox::always_allowed(self.config_dir),
        );
        match gate {
            sandbox::CodeGate::Refuse(text) => return tools::Outcome::err(text),
            sandbox::CodeGate::Run => {}
            sandbox::CodeGate::Ask(reason) => {
                let Some(tool) = self.offered.iter().find(|t| t.name == sandbox::RUN_CODE) else {
                    return tools::Outcome::err("Running code is not available.");
                };
                let gate = ChatTool {
                    connector_id: Some("sandbox".into()),
                    ..tool.clone()
                };
                match self.host.approve(&gate, arguments, reason).await {
                    ApprovalDecision::Deny => {
                        return tools::Outcome::err("The user declined to run this code. Do not retry it; ask what they would like instead.")
                    }
                    // "Always" covers isolated runs; an unsandboxed one asks every time.
                    ApprovalDecision::Always if reason.is_none() => {
                        if let Err(e) = sandbox::set_always_allowed(self.config_dir, true) {
                            warn!("could not save the code-run permission: {e}");
                        }
                    }
                    ApprovalDecision::Always | ApprovalDecision::Once => {}
                }
            }
        }
        // An agent's code runs in its own environment (home, packages), kept across its chats.
        let env = self
            .agent
            .and_then(|a| sandbox::agent_env(self.config_dir, &a.id, a.code_network).ok());
        sandbox::run_in(self.config_dir, self.id, arguments, env.as_ref()).await
    }
}

/// Puts the model's question to the user and waits for the answer.
pub(super) async fn ask_user(
    host: &dyn Host,
    id: &str,
    call: &ToolCall,
    cancel: &mut watch::Receiver<bool>,
) -> tools::Outcome {
    let arguments = Value::Object(call.function.arguments.clone());
    let call_id = call.id.to_string();
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let options = arguments
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let state = host.chat_state();
    let answer = state.ask(&call_id);
    host.emit(ChatEvent::Question {
        conversation_id: id.into(),
        call_id: call_id.clone(),
        question,
        options,
    });
    tokio::select! {
        _ = cancelled(cancel) => {
            state.forget_question(&call_id);
            tools::Outcome::err("The user dismissed the question.")
        }
        answer = answer => match answer {
            Ok(text) => tools::Outcome { text, is_error: false },
            Err(_) => tools::Outcome::err("The user dismissed the question."),
        }
    }
}
