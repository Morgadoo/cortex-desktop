//! An independent review of a change by another model, and the handlers that need one: retiring a tool and requesting a capability.

use super::*;

/// One line from a reviewer that did not write what it reviews: a fresh request with none of this
/// chat in it.
/// A review by a reviewer that is not the author: the review model when one is set (a different
/// model), else the chat's model in a fresh request that holds none of this chat. One line back.
pub(super) async fn independent_review(r: &Runner<'_>, prompt: &str) -> String {
    let reviewer = match reviewer_reference(r.config_dir, &r.request.model) {
        Some(reference) => r.host.connect_model(&reference).await.ok(),
        None => None,
    };
    review_with(reviewer.as_ref().unwrap_or(r.model), prompt).await
}

/// The Reviewer model from Settings, when one is set and it is not the chat's own model; else
/// the chat's model reviews, in a fresh request.
pub(in crate::chat::turn::engine) fn reviewer_reference(
    config_dir: &std::path::Path,
    chat_model: &str,
) -> Option<String> {
    crate::chat::providers::load(config_dir)
        .review_model
        .filter(|m| m != chat_model)
}

/// One review: a fresh request holding only `prompt`, none of the chat, answered in one line.
pub(in crate::chat::turn::engine) async fn review_with(
    model: &DynModel<Completion>,
    prompt: &str,
) -> String {
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
pub(super) fn on_retire_tool<'a>(
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
            Ok(()) => {
                tools::Outcome::ok(format!("`{name}` is retired; promote_tool brings it back."))
            }
            Err(e) => tools::Outcome::err(e),
        }
    })
}

/// `request_capability`: more access for a tool, for a time, with a reason; it always asks.
#[allow(unused_variables)]
pub(super) fn on_request_capability<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        use crate::chat::script_tools;
        let refuse = |text: &str| tools::Outcome::err(text);
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
                Err(e) => refuse(&e.to_string()),
            },
        }
    })
}
