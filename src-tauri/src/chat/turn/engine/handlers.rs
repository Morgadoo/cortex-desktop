//! The engine's own tools, each a handler found by its name: adding one means writing its
//! handler here and adding it to [`table`]. Calls to anything else go to the connectors.

use std::collections::HashMap;
use std::sync::OnceLock;

use futures::future::BoxFuture;

use super::*;

/// Runs one call: the runner, the call, its arguments, and whether the user already approved it
/// under the agent's "ask first" policy.
pub(in crate::chat::turn::engine) type Handler =
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
pub(in crate::chat::turn::engine) fn find(name: &str) -> Option<Handler> {
    static TABLE: OnceLock<HashMap<&'static str, Handler>> = OnceLock::new();
    TABLE.get_or_init(table).get(name).copied()
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
            return tools::Outcome::err("Give the notification a title.");
        }
        r.host.notify(&title, &text("body"));
        tools::Outcome::ok("Notification shown.")
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
                Err(why) => return tools::Outcome::err(why),
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

mod context_tools;
mod making;
mod ranking;
mod review;
mod team;
mod written_tools;
pub(in crate::chat::turn::engine) use context_tools::*;
use making::*;
pub(in crate::chat::turn::engine) use ranking::*;
pub(in crate::chat::turn::engine) use review::*;
use team::*;
use written_tools::*;
