//! Asking the user to approve a tool call: the card on the page, the wait for an answer, and
//! the audit entry. In the bottom layer, so anything that runs a call (the engine, a coding
//! agent's back end) asks the same way.

use std::time::Duration;

use cortex_kernel::tool::ChatTool;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::tools::approvals;
use crate::tools::registry::ApprovalDecision;

/// How long a call waits for the user to approve it.
const APPROVAL_WAIT: Duration = Duration::from_secs(15 * 60);

/// Asks the page to approve a call, with the payload the registry's own gate uses, so one
/// approval card serves both.
pub(crate) async fn request_approval(
    app: &AppHandle,
    tool: &ChatTool,
    input: &Value,
) -> ApprovalDecision {
    request_approval_because(app, tool, input, None).await
}

/// Asks with a reason Auto mode gave, shown on the card.
pub(crate) async fn request_approval_because(
    app: &AppHandle,
    tool: &ChatTool,
    input: &Value,
    auto_reason: Option<&str>,
) -> ApprovalDecision {
    let reason = cortex_kernel::approval::risk(tool, input)
        .map(|why| json!({ "kind": "risky", "sources": [], "detail": why }))
        .or_else(|| auto_reason.map(|why| json!({ "kind": "auto", "sources": [], "detail": why })));
    let approvals = tauri::Manager::state::<approvals::PendingApprovals>(app);
    let (request_id, rx) = approvals.register();
    let _ = app.emit(
        "tool:approval_request",
        json!({
            "request_id": request_id,
            "connector_id": tool.connector_id,
            "tool_name": tool.tool_name,
            "input": input,
            "reason": reason,
        }),
    );
    let decision = match tokio::time::timeout(APPROVAL_WAIT, rx).await {
        Ok(Ok(decision)) => decision,
        _ => {
            approvals.cancel(&request_id);
            let _ = app.emit(
                "tool:approval_resolved",
                json!({ "request_id": request_id, "approved": false }),
            );
            ApprovalDecision::Deny
        }
    };
    if let Ok(dir) = crate::settings::get_config_dir(app) {
        crate::chat::audit::record(
            &dir,
            crate::chat::audit::Entry {
                at: chrono::Utc::now(),
                kind: "approval".into(),
                conversation: String::new(),
                agent: None,
                tool: tool.name.clone(),
                arguments: crate::chat::audit::short(&input.to_string()),
                outcome: match decision {
                    ApprovalDecision::Deny => "denied",
                    ApprovalDecision::Once => "approved",
                    ApprovalDecision::Always => "approved always",
                }
                .into(),
                detail: auto_reason.map(str::to_string).unwrap_or_default(),
                millis: 0,
            },
        );
    }
    decision
}
