//! Handlers for the tools the assistant writes: creating, running and promoting them (with trials against the eval suites), and proposing changes.

use super::*;
use crate::chat::error::ChatError;

/// `create_tool`: writing and testing a candidate is free; it is not offered until promoted.
#[allow(unused_variables)]
pub(super) fn on_create_tool<'a>(
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
        let name = arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let approved = script_tools::load(r.config_dir, &name)
            .is_some_and(|t| t.status != script_tools::Status::Candidate);
        if approved && mode != ApprovalMode::Plan && !mode.unattended() && !asked {
            let gate = ChatTool {
                name: script_tools::CREATE_TOOL.into(),
                description: String::new(),
                parameters: Value::Null,
                connector_id: None,
                tool_name: script_tools::CREATE_TOOL.into(),
                read_only: false,
            };
            if r.host
                .approve(
                    &gate,
                    &arguments,
                    Some(
                        "rewriting an approved tool takes it out of use until it is approved again",
                    ),
                )
                .await
                == ApprovalDecision::Deny
            {
                return tools::Outcome {
                    text: format!(
                        "The user kept `{name}` as it is. Use it, or describe the change."
                    ),
                    is_error: true,
                };
            }
        }
        if mode == ApprovalMode::Plan || mode.unattended() {
            let name = arguments
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if script_tools::load(r.config_dir, name.trim())
                .is_some_and(|t| t.status != script_tools::Status::Candidate)
            {
                return tools::Outcome::err(format!("`{name}` is an approved tool; rewriting it needs the user, who is not here. Use it as it is, or describe the change."));
            }
        }
        match script_tools::create(r.config_dir, r.id, &arguments).await {
            Ok(text) => tools::Outcome {
                text,
                is_error: false,
            },
            Err(text) => tools::Outcome {
                text: text.to_string(),
                is_error: true,
            },
        }
    })
}

/// `run_script`: computes only, so it runs in every mode.
#[allow(unused_variables)]
pub(super) fn on_run_script<'a>(
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
                text: text.to_string(),
                is_error: true,
            },
        }
    })
}

/// `promote_tool`: always asks the user, whatever the mode, showing the script, the description
/// the model will read and the access asked for. Nobody can approve in a scheduled run or in Plan
/// mode, so there it is refused.
#[allow(unused_variables)]
pub(super) fn on_promote_tool<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        let refuse = |text: &str| tools::Outcome::err(text);
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
            let mut tried =
                script_tools::load(&dir, name).ok_or_else(|| ChatError::Invalid("gone".into()))?;
            tried.granted = script_tools::Capabilities::default();
            script_tools::save(&dir, &tried)
        })
        .await
        {
            Ok(scores) => scores,
            Err(why) => return refuse(&why.to_string()),
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
                Ok(text) => tools::Outcome::ok(format!("{text}{}", scores_note(&scores))),
                Err(e) => refuse(&e.to_string()),
            },
        }
    })
}

/// The user's suites that have something to check.
pub(super) fn runnable_suites(config_dir: &std::path::Path) -> Vec<String> {
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
pub(super) async fn trial<F, Fut>(r: &Runner<'_>, apply: F) -> Result<Option<Value>, ChatError>
where
    F: FnOnce(std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = Result<(), ChatError>>,
{
    use crate::chat::evals;
    let suites = runnable_suites(r.config_dir);
    if suites.is_empty() {
        return Ok(None);
    }
    let folder = evals::trial_folder(r.config_dir).map_err(ChatError::Invalid)?;
    let host: Arc<dyn Host> = Arc::new(evals::InFolder::new(r.host.clone(), folder.dir.clone()));
    let mut before = Vec::new();
    for suite in &suites {
        before.push(
            evals::run_suite(&host, suite, &r.request.model, &evals::Without::default())
                .await
                .map_err(ChatError::Invalid)?,
        );
    }
    apply(folder.dir.clone()).await?;
    let mut scores = Vec::new();
    for (suite, base) in suites.iter().zip(&before) {
        let after = evals::run_suite(&host, suite, &r.request.model, &evals::Without::default())
            .await
            .map_err(ChatError::Invalid)?;
        let mut comparison = evals::compare(base, &after);
        if !comparison.holds() {
            // Models answer differently from run to run: one worse run is checked once more
            // before the change is held to have made the suite worse.
            let again =
                evals::run_suite(&host, suite, &r.request.model, &evals::Without::default())
                    .await
                    .map_err(ChatError::Invalid)?;
            let second = evals::compare(base, &again);
            if second.holds() {
                comparison = second;
            }
        }
        if !comparison.holds() {
            log::info!(
                "trial: `{suite}` got worse ({:.2} to {:.2}; worse: {})",
                comparison.before,
                comparison.after,
                comparison.worse.join(", ")
            );
            return Err(ChatError::Invalid(
                "Tried against the user's eval suites, it made one worse, so it was not offered. \
                 Leave it, or propose something else."
                    .into(),
            ));
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
pub(super) fn scores_note(scores: &Option<Value>) -> String {
    match scores {
        Some(_) => " It was measured against the user's eval suites, and none got worse.".into(),
        None => String::new(),
    }
}

/// `propose_change`: a reviewer that did not write the proposal reads it first, then the user
/// decides, in every mode; nobody can in Plan mode or a scheduled run, so there it is refused.
#[allow(unused_variables)]
pub(super) fn on_propose_change<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::proposals;
        let refuse = |text: &str| tools::Outcome::err(text);
        let proposal = match proposals::parse(&arguments) {
            Ok(p) => p,
            Err(e) => return refuse(&e.to_string()),
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
            Err(why) => return refuse(&why.to_string()),
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
                Ok(text) => tools::Outcome::ok(format!(
                    "{text} (Reviewer: {verdict}){}",
                    scores_note(&scores)
                )),
                Err(e) => refuse(&e.to_string()),
            },
        }
    })
}
