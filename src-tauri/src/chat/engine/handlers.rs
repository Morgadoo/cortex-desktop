//! The engine's own tools, each a handler found by its name: adding one means writing its
//! handler here and adding it to [`table`]. Calls to anything else go to the connectors.

use std::collections::HashMap;
use std::sync::OnceLock;

use futures::future::BoxFuture;

use super::*;

/// Runs one call: the runner, the call, its arguments, and whether the user already approved it
/// under the agent's "ask first" policy.
pub(super) type Handler =
    for<'a> fn(&'a Runner<'a>, &'a ToolCall, Value, bool) -> BoxFuture<'a, tools::Outcome>;

fn table() -> HashMap<&'static str, Handler> {
    let mut t: HashMap<&'static str, Handler> = HashMap::new();
    t.insert(builtins::DELEGATE, on_delegate);
    t.insert(agents::MESSAGE_AGENT, on_message_agent);
    t.insert(crate::chat::org::MANAGE_AGENTS, on_manage_agents);
    t.insert(deferred::FIND_TOOLS, on_find_tools);
    t.insert(agents::NOTEBOOK_TOOL, on_notebook);
    t.insert(builtins::CANVAS, on_canvas);
    t.insert(sandbox::RUN_CODE, on_run_code);
    t.insert(terminal::START, on_terminal);
    t.insert(terminal::READ, on_terminal);
    t.insert(terminal::STOP, on_terminal);
    t.insert(crate::chat::webpage::BROWSER_ACT, on_browser_act);
    t.insert(crate::chat::webpage::BROWSER_VIEW, on_browser_view);
    t.insert(media::GENERATE_IMAGE, on_generate_image);
    t.insert(crate::chat::animation::MAKE_ANIMATION, on_make_animation);
    t.insert(crate::chat::videogen::GENERATE_VIDEO, on_generate_video);
    t.insert(builtins::NOTIFY, on_notify);
    t.insert(READ_TOOL_OUTPUT, on_read_tool_output);
    for name in builtins::NAMES {
        t.insert(name, on_builtin);
    }
    t.insert(crate::chat::script_tools::CREATE_TOOL, on_create_tool);
    t.insert(crate::chat::script_tools::RUN_SCRIPT, on_run_script);
    t.insert(crate::chat::script_tools::PROMOTE_TOOL, on_promote_tool);
    t.insert(crate::chat::proposals::PROPOSE_CHANGE, on_propose_change);
    t.insert(CONTEXT_STATUS, on_context_status);
    t.insert(crate::chat::script_tools::RETIRE_TOOL, on_retire_tool);
    t.insert(
        crate::chat::script_tools::REQUEST_CAPABILITY,
        on_request_capability,
    );
    t.insert(UNLOAD_TOOLS, on_unload_tools);
    t
}

/// The handler for `name`, if the engine runs that tool itself.
pub(super) fn find(name: &str) -> Option<Handler> {
    static TABLE: OnceLock<HashMap<&'static str, Handler>> = OnceLock::new();
    TABLE.get_or_init(table).get(name).copied()
}

#[allow(unused_variables, unused_mut)]
fn on_delegate<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        match subagents::parse_tasks(&arguments) {
            Ok(tasks) => {
                let reports = subagents::run_all(
                    r.host,
                    r.config_dir,
                    &r.request.model,
                    r.offered,
                    r.id,
                    &call.id.to_string(),
                    tasks,
                    subagents::Mode::parse(&arguments),
                )
                .await;
                tools::Outcome {
                    text: subagents::render(&reports),
                    is_error: reports.iter().all(|r| r.status != "done"),
                }
            }
            Err(e) => tools::Outcome {
                text: e,
                is_error: true,
            },
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_message_agent<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return message_agent(
            r.host,
            r.config_dir,
            r.id,
            r.agent,
            &arguments,
            r.workdir.as_deref(),
            r.request.approval,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_manage_agents<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.manage_agents(&arguments, asked).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_find_tools<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let semantic = semantic_tool_ranking(r.config_dir, r.offered, query).await;
        tools::Outcome {
            text: deferred::find_with(r.offered, &arguments, semantic).0,
            is_error: false,
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_notebook<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let Some(agent_id) = store::load_conversation(r.config_dir, r.id)
            .ok()
            .and_then(|c| c.agent_id)
        else {
            return tools::Outcome {
                text: "Only agents keep a notebook.".into(),
                is_error: true,
            };
        };
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match agents::save_notebook(r.config_dir, &agent_id, content) {
            Ok(()) => tools::Outcome {
                text: "Notebook saved.".into(),
                is_error: false,
            },
            Err(e) => tools::Outcome {
                text: e,
                is_error: true,
            },
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_canvas<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let text = |k: &str| arguments.get(k).and_then(Value::as_str).map(str::to_string);
        // A bigger app comes as several files.
        let files: Option<Vec<canvas::AppFile>> = arguments
            .get("files")
            .and_then(|f| serde_json::from_value(f.clone()).ok())
            .filter(|f: &Vec<canvas::AppFile>| !f.is_empty());
        let content = match files {
            Some(files) => match canvas::bundle(&files) {
                Ok(c) => Some(c),
                Err(e) => {
                    return tools::Outcome {
                        text: e,
                        is_error: true,
                    }
                }
            },
            None => text("content").filter(|c| !c.trim().is_empty()),
        };
        let Some(content) = content else {
            return tools::Outcome {
                text: "Give the canvas content.".into(),
                is_error: true,
            };
        };
        let id = text("id").filter(|i| !i.trim().is_empty());
        let kind = if arguments.get("files").is_some() {
            Some("app".to_string())
        } else {
            text("kind")
        };
        match canvas::write_kind(r.config_dir, r.id, id.as_deref(), text("title").as_deref(), &content, "assistant", kind.as_deref()) {
            Ok(c) => tools::Outcome {
                text: serde_json::json!({ "id": c.id, "title": c.title, "kind": c.kind, "version": c.versions.len() }).to_string(),
                is_error: false,
            },
            Err(e) => tools::Outcome { text: e, is_error: true },
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_run_code<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.run_code(&arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_terminal<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return r.terminal(name, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_browser_act<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        // Using a page can change things (submit, sign up, post), so it asks unless the
        // chat runs on its own by choice.
        if !asked
            && !matches!(
                r.request.approval,
                ApprovalMode::Auto | ApprovalMode::Autonomous
            )
        {
            if r.request.approval.unattended() {
                return tools::Outcome { text: "Using web pages needs the user's approval, and nobody is watching this run.".into(), is_error: true };
            }
            let gate = ChatTool {
                name: name.into(),
                description: String::new(),
                parameters: Value::Null,
                connector_id: Some("browser".into()),
                tool_name: name.into(),
                read_only: false,
            };
            if matches!(
                r.host.approve(&gate, &arguments, None).await,
                ApprovalDecision::Deny
            ) {
                return tools::Outcome {
                    text:
                        "The user declined this. Do not retry it; ask what they would like instead."
                            .into(),
                    is_error: true,
                };
            }
        }
        return crate::chat::webpage::act(
            r.config_dir,
            r.id,
            r.agent.map(|a| a.id.as_str()),
            &arguments,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_browser_view<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::webpage::view(
            r.config_dir,
            r.id,
            r.agent.map(|a| a.id.as_str()),
            &arguments,
        )
        .await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_generate_image<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return media::generate(r.config_dir, r.id, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_make_animation<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::animation::make(r.config_dir, r.id, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_generate_video<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        return crate::chat::videogen::generate(r.config_dir, r.id, &arguments).await;
    })
}

#[allow(unused_variables, unused_mut)]
fn on_notify<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let text = |k: &str| {
            arguments
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(240)
                .collect::<String>()
        };
        let title = text("title");
        if title.trim().is_empty() {
            return tools::Outcome {
                text: "Give the notification a title.".into(),
                is_error: true,
            };
        }
        r.host.notify(&title, &text("body"));
        tools::Outcome {
            text: "Notification shown.".into(),
            is_error: false,
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_read_tool_output<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        let call_id = arguments
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        match read_tool_output(r.config_dir, r.id, call_id, offset) {
            Ok(text) => tools::Outcome {
                text,
                is_error: false,
            },
            Err(text) => tools::Outcome {
                text,
                is_error: true,
            },
        }
    })
}

#[allow(unused_variables, unused_mut)]
fn on_builtin<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let name = call.function.name.as_str();
        // A saved skill changes how every later chat works: like a tool's promotion, it is
        // reviewed and measured, and asks the user in every mode, Auto and Full auto included,
        // unless the agent's policy already did. It is measured either way.
        let mut skill_scores = None;
        if name == builtins::SAVE_SKILL {
            match r.request.approval {
                ApprovalMode::Plan => {
                    return tools::Outcome {
                        text: "Plan mode changes nothing: describe the skill you would save."
                            .into(),
                        is_error: true,
                    }
                }
                mode if mode.unattended() => return tools::Outcome {
                    text:
                        "Nobody is watching this run to approve a new skill; describe it instead."
                            .into(),
                    is_error: true,
                },
                _ => {}
            }
            let draft = &arguments;
            skill_scores = match trial(r, |dir| async move {
                let outcome = builtins::run(&dir, name, draft).await;
                if outcome.is_error {
                    Err(outcome.text)
                } else {
                    Ok(())
                }
            })
            .await
            {
                Ok(scores) => scores,
                Err(why) => {
                    return tools::Outcome {
                        text: why,
                        is_error: true,
                    }
                }
            };
            if !asked {
                let verdict = independent_review(
                    r,
                    &format!(
                        "You review a skill an AI assistant wrote for itself: instructions it will \
                         follow in every later chat where they apply. You did not write it. Check \
                         that it is general, correct, safe, and not a near copy of a common default.\n\n{}\n\n\
                         Reply with one line: SUPPORT: and a reason, or DOUBT: and a reason, under 25 words.",
                        serde_json::to_string_pretty(&arguments).unwrap_or_default()
                    ),
                )
                .await;
                let mut card = arguments.clone();
                if let Some(map) = card.as_object_mut() {
                    map.insert("review".into(), Value::String(verdict.clone()));
                    if let Some(scores) = &skill_scores {
                        map.insert("evals".into(), scores.clone());
                    }
                }
                let gate = ChatTool {
                    name: name.into(),
                    description: String::new(),
                    parameters: Value::Null,
                    connector_id: None,
                    tool_name: name.into(),
                    read_only: false,
                };
                if r.host.approve(&gate, &card, Some(&verdict)).await == ApprovalDecision::Deny {
                    return tools::Outcome {
                        text: "The user declined this skill. Do not save it again unless they ask."
                            .into(),
                        is_error: true,
                    };
                }
            }
        } else if builtins::effect(name) == builtins::Effect::Standing {
            if let Err(refused) = r
                .confirm_change(
                    name,
                    None,
                    &arguments,
                    asked,
                    "Plan mode changes nothing: describe what you would schedule or save, and the user can switch modes to go ahead.",
                    "Nobody is watching this scheduled run to approve scheduling tasks or saving skills; describe what you would have done instead.",
                )
                .await
            {
                return refused;
            }
        }
        let mut arguments = arguments;
        // A follow-up in this chat: the task needs to know which one.
        if name == builtins::SCHEDULE_TASK {
            if let Some(map) = arguments.as_object_mut() {
                map.insert("reply_in".into(), Value::String(r.id.to_string()));
            }
        }
        // A saved fact or skill keeps a link to the chat it came from.
        if name == builtins::REMEMBER || name == builtins::SAVE_SKILL {
            if let Some(map) = arguments.as_object_mut() {
                map.insert("conversation".into(), Value::String(r.id.to_string()));
            }
        }
        // Scoped memory: the chat's project and agent, which the model never chooses.
        if name == builtins::REMEMBER || name == builtins::MEMORY_SEARCH {
            let project = store::load_conversation(r.config_dir, r.id)
                .ok()
                .and_then(|c| c.project_id);
            if let Some(map) = arguments.as_object_mut() {
                map.remove("_project");
                map.remove("_agent");
                if let Some(project) = project {
                    map.insert("_project".into(), Value::String(project));
                }
                if let Some(agent) = r.agent {
                    map.insert("_agent".into(), Value::String(agent.id.clone()));
                }
            }
        }
        let mut outcome = builtins::run(r.config_dir, name, &arguments).await;
        if !outcome.is_error {
            outcome.text.push_str(&scores_note(&skill_scores));
        }
        // Attribution: this run used the skill it opened.
        if name == builtins::USE_SKILL && !outcome.is_error {
            if let Some(skill) = arguments.get("name").and_then(Value::as_str) {
                crate::chat::traces::attribute(r.config_dir, r.run, "skill", skill, true);
            }
        }
        // And the knowledge page it read.
        if name == builtins::KNOWLEDGE_READ && !outcome.is_error {
            let key = arguments.get("page").and_then(Value::as_str).unwrap_or("");
            let pages = crate::chat::knowledge::load(r.config_dir);
            if let Some(page) = crate::chat::knowledge::find(&pages, key) {
                crate::chat::traces::used(r.config_dir, "page", &page.id);
                crate::chat::traces::attribute(r.config_dir, r.run, "page", &page.id, true);
            }
        }
        outcome
    })
}

/// `create_tool`: writing and testing a candidate is free; it is not offered until promoted.
#[allow(unused_variables)]
fn on_create_tool<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        // Rewriting an approved tool sends it back to candidate, which takes it out of every
        // chat: a change nobody can approve in Plan mode or in a run nobody watches (evals too).
        let mode = r.request.approval;
        if mode == ApprovalMode::Plan || mode.unattended() {
            let name = arguments
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if script_tools::load(r.config_dir, name.trim())
                .is_some_and(|t| t.status != script_tools::Status::Candidate)
            {
                return tools::Outcome {
                    text: format!("`{name}` is an approved tool; rewriting it needs the user, who is not here. Use it as it is, or describe the change."),
                    is_error: true,
                };
            }
        }
        match script_tools::create(r.config_dir, r.id, &arguments).await {
            Ok(text) => tools::Outcome {
                text,
                is_error: false,
            },
            Err(text) => tools::Outcome {
                text,
                is_error: true,
            },
        }
    })
}

/// `run_script`: computes only, so it runs in every mode.
#[allow(unused_variables)]
fn on_run_script<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        match crate::chat::script_tools::run_once(r.config_dir, &arguments).await {
            Ok(text) => tools::Outcome {
                text,
                is_error: false,
            },
            Err(text) => tools::Outcome {
                text,
                is_error: true,
            },
        }
    })
}

/// `promote_tool`: always asks the user, whatever the mode, showing the script, the description
/// the model will read and the access asked for. Nobody can approve in a scheduled run or in Plan
/// mode, so there it is refused.
#[allow(unused_variables)]
fn on_promote_tool<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        let refuse = |text: &str| tools::Outcome {
            text: text.into(),
            is_error: true,
        };
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(tool) = script_tools::load(r.config_dir, name) else {
            return refuse("There is no tool by that name; write it with create_tool first.");
        };
        match r.request.approval {
            ApprovalMode::Plan => {
                return refuse("Plan mode changes nothing: say which tool you would promote, and the user can switch modes.")
            }
            mode if mode.unattended() => {
                return refuse("Nobody is watching this run to approve a new tool; promote it in a chat.")
            }
            _ => {}
        }
        let gate = ChatTool {
            name: script_tools::PROMOTE_TOOL.into(),
            description: String::new(),
            parameters: Value::Null,
            connector_id: None,
            tool_name: script_tools::PROMOTE_TOOL.into(),
            read_only: false,
        };
        // The author is not its only reviewer: a fresh model, with none of this chat, reads it first.
        let verdict = independent_review(
            r,
            &format!(
                "You review a tool an AI assistant wrote for itself, before its user decides whether \
                 to make it available in every chat. You did not write it. Check that the script does \
                 what the description says and nothing else, that the access it asks for is the least \
                 it needs, and that it cannot leak the user's data.\n\n{}\n\nReply with one line: \
                 SUPPORT: and a reason, or DOUBT: and a reason, under 25 words.",
                script_tools::review(&tool)
            ),
        )
        .await;
        let days = arguments.get("days").and_then(Value::as_i64);
        // Tried first, in a copy of the data, with nothing granted: it is not approved yet.
        let scores = match trial(r, |dir| async move {
            script_tools::promote(&dir, name, None).await?;
            let mut tried = script_tools::load(&dir, name).ok_or("gone")?;
            tried.granted = script_tools::Capabilities::default();
            script_tools::save(&dir, &tried)
        })
        .await
        {
            Ok(scores) => scores,
            Err(why) => return refuse(&why),
        };
        let mut card = script_tools::review(&tool);
        card["review"] = Value::String(verdict.clone());
        if let Some(scores) = &scores {
            card["evals"] = scores.clone();
        }
        if let Some(days) = days {
            card["accessForDays"] = days.into();
        }
        match r.host.approve(&gate, &card, Some(&verdict)).await {
            ApprovalDecision::Deny => refuse(
                "The user declined this tool. Do not promote it again unless they ask; describe what it would do instead.",
            ),
            _ => match script_tools::promote_as_shown(r.config_dir, &tool, days).await {
                Ok(text) => tools::Outcome {
                    text: format!("{text}{}", scores_note(&scores)),
                    is_error: false,
                },
                Err(e) => refuse(&e),
            },
        }
    })
}

/// The user's suites that have something to check.
fn runnable_suites(config_dir: &std::path::Path) -> Vec<String> {
    use crate::chat::evals;
    evals::suites(config_dir)
        .into_iter()
        .filter(|s| {
            evals::cases(config_dir, s)
                .iter()
                .any(|c| !c.needs_expectation)
        })
        .collect()
}

/// Measured against a set the agent does not control, before the user is asked: with the
/// user's eval suites, the change is made in a throwaway copy of the data folder, and each suite
/// runs there before and after it. The live data is never touched, so a stopped turn, a declined
/// change or two trials at once leave nothing behind, and trial runs never become a baseline.
/// The scores go on the approval card; a change that makes any suite worse is never offered.
/// The model is told only that, not the scores or the cases, so it cannot fit itself to the
/// suites. `Ok(None)` without suites: the user's approval stands alone.
async fn trial<F, Fut>(r: &Runner<'_>, apply: F) -> Result<Option<Value>, String>
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    use crate::chat::evals;
    let suites = runnable_suites(r.config_dir);
    if suites.is_empty() {
        return Ok(None);
    }
    let folder = evals::trial_folder(r.config_dir)?;
    let host: Arc<dyn Host> = Arc::new(evals::InFolder::new(r.host.clone(), folder.dir.clone()));
    let mut before = Vec::new();
    for suite in &suites {
        before.push(
            evals::run_suite(&host, suite, &r.request.model, &evals::Without::default()).await?,
        );
    }
    apply(folder.dir.clone()).await?;
    let mut scores = Vec::new();
    for (suite, base) in suites.iter().zip(&before) {
        let after =
            evals::run_suite(&host, suite, &r.request.model, &evals::Without::default()).await?;
        let comparison = evals::compare(base, &after);
        if !comparison.holds() {
            log::info!(
                "trial: `{suite}` got worse ({:.2} to {:.2}; worse: {})",
                comparison.before,
                comparison.after,
                comparison.worse.join(", ")
            );
            return Err(
                "Tried against the user's eval suites, it made one worse, so it was not offered. \
                 Leave it, or propose something else."
                    .into(),
            );
        }
        scores.push(serde_json::json!({
            "suite": suite,
            "before": comparison.before,
            "after": comparison.after,
            "better": comparison.better,
        }));
    }
    Ok(Some(Value::Array(scores)))
}

/// What the result says: that it was measured, not the scores (those are for the user).
fn scores_note(scores: &Option<Value>) -> String {
    match scores {
        Some(_) => " It was measured against the user's eval suites, and none got worse.".into(),
        None => String::new(),
    }
}

/// `propose_change`: a reviewer that did not write the proposal reads it first, then the user
/// decides, in every mode; nobody can in Plan mode or a scheduled run, so there it is refused.
#[allow(unused_variables)]
fn on_propose_change<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::proposals;
        let refuse = |text: &str| tools::Outcome {
            text: text.into(),
            is_error: true,
        };
        let proposal = match proposals::parse(&arguments) {
            Ok(p) => p,
            Err(e) => return refuse(&e),
        };
        match r.request.approval {
            ApprovalMode::Plan => {
                return refuse("Plan mode changes nothing: include the proposal in your plan.")
            }
            mode if mode.unattended() => {
                return refuse(
                    "Nobody is watching this run to approve a change; propose it in a chat.",
                )
            }
            _ => {}
        }
        // The independent review: a fresh request, with none of this chat in it.
        let verdict = independent_review(r, &proposals::review_prompt(&proposal, &arguments)).await;
        let gate = ChatTool {
            name: proposals::PROPOSE_CHANGE.into(),
            description: String::new(),
            parameters: Value::Null,
            connector_id: None,
            tool_name: proposals::PROPOSE_CHANGE.into(),
            read_only: false,
        };
        let change = &proposal.change;
        let scores = match trial(r, |dir| async move {
            proposals::apply(&dir, r.id, change).map(drop)
        })
        .await
        {
            Ok(scores) => scores,
            Err(why) => return refuse(&why),
        };
        let card = serde_json::json!({
            "summary": proposal.summary,
            "evidence": proposal.evidence,
            "change": arguments.get("change"),
            "review": verdict,
            "evals": scores,
        });
        match r.host.approve(&gate, &card, Some(&verdict)).await {
            ApprovalDecision::Deny => {
                refuse("The user declined this change. Do not propose it again unless they ask.")
            }
            _ => match proposals::apply(r.config_dir, r.id, &proposal.change) {
                Ok(text) => tools::Outcome {
                    text: format!("{text} (Reviewer: {verdict}){}", scores_note(&scores)),
                    is_error: false,
                },
                Err(e) => refuse(&e),
            },
        }
    })
}

pub(super) const CONTEXT_STATUS: &str = "context_status";
pub(super) const UNLOAD_TOOLS: &str = "unload_tools";

pub(super) fn context_status_tool() -> ChatTool {
    ChatTool {
        name: CONTEXT_STATUS.into(),
        description:
            "How full your context is: the model's window, what the last request carried, \
            the parts of your instructions and their sizes, and the tools loaded on demand. Use it \
            before a long task to decide what to unload or summarize."
                .into(),
        parameters: serde_json::json!({ "type": "object", "properties": {} }),
        connector_id: None,
        tool_name: CONTEXT_STATUS.into(),
        read_only: true,
    }
}

pub(super) fn unload_tools_tool() -> ChatTool {
    ChatTool {
        name: UNLOAD_TOOLS.into(),
        description: "Unload tools you loaded with find_tools and no longer need, to free room; \
            find_tools brings them back."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "names": { "type": "array", "items": { "type": "string" } } },
            "required": ["names"]
        }),
        connector_id: None,
        tool_name: UNLOAD_TOOLS.into(),
        read_only: true,
    }
}

/// `context_status`: the kernel's `budget_status()`.
#[allow(unused_variables)]
fn on_context_status<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let Ok(c) = store::load_conversation(r.config_dir, r.id) else {
            return tools::Outcome {
                text: "The conversation could not be read.".into(),
                is_error: true,
            };
        };
        let window = context::context_window(&r.request.model);
        let status = serde_json::json!({
            "window": window,
            "lastRequest": c.usage.context_tokens,
            "percentUsed": (c.usage.context_tokens as f64 / window.max(1) as f64 * 100.0).round(),
            "summarizesAtPercent": (context::COMPACT_AT * 100.0).round(),
            "instructions": c.prompt_parts,
            "toolsLoadedOnDemand": c.loaded_tools,
            "earlierMessagesSummarized": c.summary.as_ref().map_or(0, |s| s.upto),
        });
        tools::Outcome {
            text: serde_json::to_string_pretty(&status).unwrap_or_default(),
            is_error: false,
        }
    })
}

/// `unload_tools`: the kernel's `unload(ref)`; the turn drops them from what it sends.
#[allow(unused_variables)]
fn on_unload_tools<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let names: Vec<String> = arguments
            .get("names")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        tools::Outcome {
            text: serde_json::json!({ "unloaded": names }).to_string(),
            is_error: false,
        }
    })
}

/// One line from a reviewer that did not write what it reviews: a fresh request with none of this
/// chat in it.
/// A review by a reviewer that is not the author: the review model when one is set (a different
/// model), else the chat's model in a fresh request that holds none of this chat. One line back.
async fn independent_review(r: &Runner<'_>, prompt: &str) -> String {
    let reviewer = match reviewer_reference(r.config_dir, &r.request.model) {
        Some(reference) => r.host.connect_model(&reference).await.ok(),
        None => None,
    };
    review_with(reviewer.as_ref().unwrap_or(r.model), prompt).await
}

/// The Reviewer model from Settings, when one is set and it is not the chat's own model; else
/// the chat's model reviews, in a fresh request.
pub(super) fn reviewer_reference(config_dir: &std::path::Path, chat_model: &str) -> Option<String> {
    crate::chat::providers::load(config_dir)
        .review_model
        .filter(|m| m != chat_model)
}

/// One review: a fresh request holding only `prompt`, none of the chat, answered in one line.
pub(super) async fn review_with(model: &DynModel<Completion>, prompt: &str) -> String {
    let review = rig_core::completion::CompletionRequest::new(rig_core::completion::Message::user(
        prompt.to_string(),
    ))
    .max_tokens(200u64);
    match tokio::time::timeout(std::time::Duration::from_secs(60), model.call(review)).await {
        Ok(Ok(response)) if !response.text().trim().is_empty() => response
            .text()
            .trim()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        _ => "The reviewer could not be reached; judge it yourself.".to_string(),
    }
}

/// `retire_tool`: hiding a tool the assistant wrote is a change, under the approval mode.
#[allow(unused_variables)]
fn on_retire_tool<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Err(refused) = r
            .confirm_change(
                script_tools::RETIRE_TOOL,
                None,
                &arguments,
                asked,
                "Plan mode changes nothing: say which tool you would retire.",
                "Nobody is watching this run to approve retiring a tool.",
            )
            .await
        {
            return refused;
        }
        match script_tools::retire(r.config_dir, &name) {
            Ok(()) => tools::Outcome {
                text: format!("`{name}` is retired; promote_tool brings it back."),
                is_error: false,
            },
            Err(e) => tools::Outcome {
                text: e,
                is_error: true,
            },
        }
    })
}

/// `request_capability`: more access for a tool, for a time, with a reason; it always asks.
#[allow(unused_variables)]
fn on_request_capability<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        let refuse = |text: &str| tools::Outcome {
            text: text.into(),
            is_error: true,
        };
        if r.request.approval == ApprovalMode::Plan || r.request.approval.unattended() {
            return refuse("Nobody can grant access in this mode; ask in a chat.");
        }
        let strings = |key: &str| -> Vec<String> {
            arguments
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        };
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let wanted = script_tools::Capabilities {
            http_hosts: strings("http_hosts"),
            read_prefixes: strings("read_prefixes"),
            write_prefixes: strings("write_prefixes"),
            credentials: strings("credentials"),
            tools: strings("tools"),
        };
        let days = arguments.get("days").and_then(Value::as_i64);
        let Some(before) = script_tools::load(r.config_dir, name) else {
            return refuse("There is no tool by that name.");
        };
        let gate = ChatTool {
            name: script_tools::REQUEST_CAPABILITY.into(),
            description: String::new(),
            parameters: Value::Null,
            connector_id: None,
            tool_name: script_tools::REQUEST_CAPABILITY.into(),
            read_only: false,
        };
        let card = serde_json::json!({
            "tool": name,
            "network": wanted.http_hosts,
            "reads": wanted.read_prefixes,
            "writes": wanted.write_prefixes,
            "credentials": wanted.credentials,
            "calls": wanted.tools,
            "forDays": days,
            "reason": arguments.get("reason"),
            "script": before.script,
            "access": script_tools::access_card(&before, &wanted),
        });
        match r
            .host
            .approve(&gate, &card, Some("a tool asks for more access"))
            .await
        {
            ApprovalDecision::Deny => {
                refuse("The user declined; the tool keeps the access it had.")
            }
            _ => match script_tools::extend(r.config_dir, &before, &wanted, days) {
                Ok(()) => tools::Outcome {
                    text: format!(
                        "`{name}` may now reach what it asked for{}.",
                        days.map_or(String::new(), |d| format!(" for {d} days"))
                    ),
                    is_error: false,
                },
                Err(e) => refuse(&e),
            },
        }
    })
}

/// The deferrable tools ranked by meaning against `query`, when the user set up an embedding
/// model: the registry's semantic search. Tool descriptions are embedded once per process.
async fn semantic_tool_ranking(
    config_dir: &std::path::Path,
    offered: &[ChatTool],
    query: &str,
) -> Option<Vec<String>> {
    use crate::chat::embeddings;
    let settings = embeddings::load(config_dir);
    if !embeddings::ready(&settings) {
        return None;
    }
    rank_by_meaning(offered, query, &settings.model, |texts| async {
        embeddings::embed(&settings, texts).await
    })
    .await
}

/// The ranking itself: `embed` turns the query and each tool's name and description into
/// vectors (with `model`'s cache), and tools come back closest in meaning first.
pub(super) async fn rank_by_meaning<F, Fut>(
    offered: &[ChatTool],
    query: &str,
    model: &str,
    embed: F,
) -> Option<Vec<String>>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Vec<f32>>, String>>,
{
    use crate::chat::embeddings;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    if query.trim().is_empty() {
        return None;
    }
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let tools: Vec<&ChatTool> = offered.iter().filter(|t| deferred::deferrable(t)).collect();
    let text = |t: &ChatTool| {
        format!(
            "{}: {}",
            t.name.replace("__", " ").replace('_', " "),
            t.description
        )
    };
    let key = |t: &ChatTool| format!("{model}\n{}", text(t));
    let missing: Vec<String> = {
        let cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        tools
            .iter()
            .filter(|t| !cache.contains_key(&key(t)))
            .map(|t| text(t))
            .collect()
    };
    let mut wanted = vec![query.to_string()];
    wanted.extend(missing.iter().cloned());
    let vectors = embed(wanted).await.ok()?;
    let (query_vector, new) = vectors.split_first()?;
    {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        for (text, vector) in missing.iter().zip(new) {
            cache.insert(format!("{model}\n{text}"), vector.clone());
        }
    }
    let cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    let mut scored: Vec<(f32, String)> = tools
        .iter()
        .filter_map(|t| {
            cache
                .get(&key(t))
                .map(|v| (embeddings::cosine(query_vector, v), t.name.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Some(scored.into_iter().map(|(_, name)| name).collect())
}
