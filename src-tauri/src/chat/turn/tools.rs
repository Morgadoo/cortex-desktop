//! The tools a conversation offers the model: the app's connectors, plus `ask_user`.
//!
//! A connector tool is named `{connector_id}__{tool}` as the web app named them, made safe for
//! every provider's name rules. Calls run through [`ToolRegistry::dispatch_with_transport`], so
//! connector readiness, per-tool policies and the untrusted-content gate all still apply. This
//! module adds the composer's approval mode on top: in "ask" mode every tool that can change
//! something waits for the user, and "accept edits" lets file edits through.

use crate::chat::error::ChatError;
use serde_json::{json, Value};
use tauri::AppHandle;

pub use cortex_kernel::approval::{
    auto_check_prompt, auto_check_verdict, needs_approval, reads_only, risk, ApprovalMode,
};

/// The ways a command can name the app's data folder: as it is, from `~` and from `$HOME`.
pub(in crate::chat::turn) fn protected_places(config_dir: &std::path::Path) -> Vec<String> {
    let full = config_dir.to_string_lossy().to_string();
    let mut out = vec![full.clone()];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        if let Ok(rest) = config_dir.strip_prefix(&home) {
            let rest = rest.to_string_lossy();
            out.push(format!("~/{rest}"));
            out.push(format!("$home/{rest}"));
            out.push(format!("${{home}}/{rest}"));
        }
    }
    out
}

/// [`risk`], and the kernel's guardrail: a change that names the app's own data (where its
/// evaluations, permissions and audit log live) is never made without the user.
pub(in crate::chat::turn) fn risk_here(
    config_dir: Option<&std::path::Path>,
    tool: &ChatTool,
    input: &Value,
) -> Option<&'static str> {
    risk(tool, input).or_else(|| {
        config_dir
            .filter(|d| cortex_kernel::approval::names_protected(tool, input, &protected_places(d)))
            .map(|_| cortex_kernel::approval::PROTECTED)
    })
}
pub use cortex_kernel::tool::{
    normalize_schema, safe_name, value_text, ChatTool, Outcome, ASK_USER,
};

use crate::chat::host::Host;
use crate::tools::connector_state::ToolPolicy;
use crate::tools::registry::{ApprovalDecision, CallContext, ToolDefinition, ToolRegistry};

fn ask_user_tool() -> ChatTool {
    ChatTool {
        name: ASK_USER.into(),
        description: "Ask the user a short clarifying question with 2 to 5 suggested answers, \
            when the request is ambiguous and a wrong guess would waste their time. The user can \
            also type their own answer. Do not use it for questions you can answer yourself."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "The question, in the user's language." },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "2 to 5 short suggested answers."
                }
            },
            "required": ["question", "options"]
        }),
        connector_id: None,
        tool_name: ASK_USER.into(),
        read_only: true,
    }
}

/// The tools of every enabled, ready connector, minus disabled ones, plus the built-ins.
/// `connectors`, when given, limits the offer to those connector ids.
pub(in crate::chat::turn) fn available(
    registry: &ToolRegistry,
    connectors: Option<&[String]>,
) -> Vec<ChatTool> {
    let mut tools = vec![ask_user_tool()];
    for manifest in registry.all_manifests() {
        if !manifest.enabled || !manifest.readiness.is_ready() {
            continue;
        }
        if connectors.is_some_and(|ids| !ids.contains(&manifest.id)) {
            continue;
        }
        for tool in manifest.tools {
            if tool.policy == ToolPolicy::Disable {
                continue;
            }
            tools.push(ChatTool {
                name: safe_name(&format!("{}__{}", manifest.id, tool.name)),
                description: tool.description.clone(),
                parameters: normalize_schema(&tool.input_schema),
                connector_id: Some(manifest.id.clone()),
                tool_name: tool.name.clone(),
                read_only: tool.read_only == Some(true),
            });
        }
    }
    // The registry's `tool_results` reader is not offered: chats take results whole and keep
    // long ones in a file of their own (`read_tool_output`).
    tools
}

pub(crate) use crate::chat::approval::request_approval_because;

/// Tools that are not registry connectors but run behind the registry's gate: the user's own
/// tools, computer use, and OpenAPI tool servers.
fn is_external(connector_id: &str) -> bool {
    connector_id == crate::chat::custom_tools::CONNECTOR
        || connector_id == crate::chat::script_tools::CONNECTOR
        || connector_id == crate::chat::computer::CONNECTOR
        || connector_id.starts_with(crate::chat::openapi::PREFIX)
}

/// Runs an [external](is_external) tool through the registry's per-tool policy and
/// untrusted-content gate, then in its own way: a script in the sandbox, the screen, or an HTTP
/// call.
async fn execute_external(
    host: &dyn Host,
    tool: &ChatTool,
    input: Value,
    approved: bool,
    context: CallContext<'_>,
) -> Outcome {
    let connector_id = tool.connector_id.as_deref().unwrap_or_default();
    let conversation_id = context.conversation.unwrap_or_default();
    let definition = ToolDefinition {
        name: tool.tool_name.clone(),
        description: tool.description.clone(),
        input_schema: tool.parameters.clone(),
        read_only: Some(tool.read_only || reads_only(tool, &input)),
        ..ToolDefinition::default()
    };
    let config_dir = host.config_dir().ok();
    let screens = host.chat_state().part::<crate::chat::computer::Screens>();
    let bench = host.chat_state().part::<crate::chat::workbench::Bench>();
    // The registry takes the connector's errors as text; the work inside is typed.
    let run = |input: Value| async move {
        let result: Result<String, ChatError> = async move {
            if connector_id == crate::chat::computer::CONNECTOR {
                return crate::chat::computer::run(&screens, conversation_id, &input).await;
            }
            let config_dir = config_dir.ok_or_else(|| {
                ChatError::Invalid("The app's data folder is not available.".into())
            })?;
            if connector_id == crate::chat::script_tools::CONNECTOR {
                let workspace = crate::chat::sandbox::workspace(&config_dir, conversation_id).ok();
                return crate::chat::script_tools::call(
                    &config_dir,
                    &tool.tool_name,
                    &input,
                    workspace.as_deref(),
                )
                .await;
            }
            if connector_id == crate::chat::custom_tools::CONNECTOR {
                crate::chat::custom_tools::run(
                    &bench,
                    &config_dir,
                    conversation_id,
                    &tool.tool_name,
                    &input,
                )
                .await
            } else {
                crate::chat::openapi::call(&config_dir, tool, &input).await
            }
        }
        .await;
        result.map_err(String::from)
    };
    match host
        .registry()
        .dispatch_external(connector_id, &definition, input, approved, context, run)
        .await
    {
        Ok(Ok(text)) => Outcome {
            text,
            is_error: false,
        },
        Ok(Err(text)) => Outcome {
            text,
            is_error: true,
        },
        Err(e) => Outcome::err(e.to_ipc_string()),
    }
}

/// Runs a connector tool call. `ask_user` is the engine's, not this function's.
pub(in crate::chat::turn) async fn execute(
    host: &dyn Host,
    tool: &ChatTool,
    input: Value,
    mode: ApprovalMode,
    conversation_id: &str,
) -> Outcome {
    // A written tool's effect is read now, not when the turn began: access granted mid-turn
    // makes it change something from that call on.
    let fresh;
    let tool = match (tool.connector_id.as_deref(), host.config_dir()) {
        (Some(crate::chat::script_tools::CONNECTOR), Ok(dir)) => {
            match crate::chat::script_tools::load(&dir, &tool.tool_name) {
                Some(written) => {
                    fresh = ChatTool {
                        read_only: !crate::chat::script_tools::has_effects(&dir, &written),
                        ..tool.clone()
                    };
                    &fresh
                }
                None => tool,
            }
        }
        _ => tool,
    };
    let Some(connector_id) = tool.connector_id.as_deref() else {
        return Outcome::err("This tool is handled by the app.");
    };
    if mode == ApprovalMode::Plan && needs_approval(tool, &input, mode) {
        return Outcome {
            text: "Plan mode: nothing is changed yet. Do not retry; finish your investigation and \
                present this step in your plan, so the user can approve the plan and switch modes."
                .into(),
            is_error: true,
        };
    }
    if mode == ApprovalMode::Unattended && needs_approval(tool, &input, mode) {
        return Outcome {
            text: "This scheduled run may not make changes, so the call was not made. Describe \
                what you would have done instead; the user can allow changes for this task."
                .into(),
            is_error: true,
        };
    }
    let config_dir = host.config_dir().ok();
    let guarded = config_dir.as_deref().is_some_and(|d| {
        cortex_kernel::approval::names_protected(tool, &input, &protected_places(d))
    });
    if mode == ApprovalMode::Autonomous {
        if let Some(why) = risk_here(config_dir.as_deref(), tool, &input) {
            return Outcome {
                text: format!(
                    "This call was not made: {why}, and nobody is watching this scheduled run to \
                     approve it. Describe what you would have run instead."
                ),
                is_error: true,
            };
        }
    }
    let approved = if guarded || needs_approval(tool, &input, mode) {
        match host
            .approve(
                tool,
                &input,
                guarded.then_some(cortex_kernel::approval::PROTECTED),
            )
            .await
        {
            ApprovalDecision::Deny => {
                return Outcome {
                    text: "The user declined this tool call. Do not retry it; ask what they \
                        would like instead."
                        .into(),
                    is_error: true,
                }
            }
            ApprovalDecision::Once | ApprovalDecision::Always => true,
        }
    } else {
        matches!(mode, ApprovalMode::Autonomous | ApprovalMode::Auto)
    };
    let context = CallContext {
        conversation: Some(conversation_id),
        whole: true,
        ..CallContext::default()
    };
    if is_external(connector_id) {
        return execute_external(host, tool, input, approved, context).await;
    }
    match host
        .registry()
        .dispatch_with_transport(
            connector_id,
            &tool.tool_name,
            input,
            approved,
            false,
            false,
            context,
        )
        .await
    {
        Ok(value) => Outcome::ok(value_text(&value)),
        Err(e) => Outcome::err(e.to_ipc_string()),
    }
}

/// The connectors set up, as the library sync takes them.
pub(crate) fn registry_connectors(
    app: &AppHandle,
) -> Vec<std::sync::Arc<dyn crate::tools::registry::Connector>> {
    tauri::Manager::state::<crate::tools::registry::ToolRegistry>(app)
        .connectors()
        .into_iter()
        .map(|c| c as std::sync::Arc<dyn crate::tools::registry::Connector>)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(connector: Option<&str>, read_only: bool) -> ChatTool {
        ChatTool {
            name: "x".into(),
            description: String::new(),
            parameters: json!({}),
            connector_id: connector.map(Into::into),
            tool_name: "x".into(),
            read_only,
        }
    }

    #[test]
    fn names_are_made_safe_for_providers() {
        assert_eq!(
            safe_name("mcp-github__search.code"),
            "mcp-github__search_code"
        );
        assert_eq!(safe_name(&"a".repeat(80)).len(), 64);
    }

    #[test]
    fn schemas_become_objects_with_properties() {
        let schema = normalize_schema(&json!({"$schema": "x"}));
        assert_eq!(schema, json!({"type": "object", "properties": {}}));
        let kept = normalize_schema(&json!({"type": "object", "properties": {"a": {}}}));
        assert_eq!(kept["properties"], json!({"a": {}}));
    }

    #[test]
    fn approval_follows_the_mode() {
        let none = json!({});
        let write = tool(Some("filesystem"), false);
        let shell = tool(Some("bash"), false);
        let read = tool(Some("filesystem"), true);
        assert!(needs_approval(&write, &none, ApprovalMode::Ask));
        // Auto's check: clear yes, a reason to ask, and unclear answers ask.
        assert_eq!(auto_check_verdict("ALLOW"), None);
        assert_eq!(auto_check_verdict("allow - fine"), None);
        assert_eq!(
            auto_check_verdict("ASK: it emails your whole team.").as_deref(),
            Some("it emails your whole team")
        );
        assert!(auto_check_verdict("Hmm, not sure").is_some());
        assert!(auto_check_prompt(
            "fix the typo",
            "filesystem__fs_write",
            &json!({"path": "a.md"})
        )
        .contains("fix the typo"));
        // Smart-approve: risky calls ask even in full auto.
        let rm = json!({"command": "cd build &&  rm   -rf  ./out"});
        assert_eq!(risk(&shell, &rm), Some("it deletes files recursively"));
        assert!(needs_approval(&shell, &rm, ApprovalMode::Auto));
        assert!(needs_approval(
            &shell,
            &json!({"args": ["git", "push --force"]}),
            ApprovalMode::AcceptEdits
        ));
        assert!(risk(&shell, &json!({"command": "ls -la && cargo test"})).is_none());
        assert!(!needs_approval(
            &shell,
            &json!({"command": "ls"}),
            ApprovalMode::Auto
        ));
        assert!(
            needs_approval(&write, &none, ApprovalMode::Plan),
            "plan mode refuses what would ask"
        );
        assert!(
            !needs_approval(&write, &none, ApprovalMode::Auto),
            "full auto asks for nothing"
        );
        assert!(!needs_approval(&write, &none, ApprovalMode::AcceptEdits));
        assert!(needs_approval(&shell, &none, ApprovalMode::AcceptEdits));
        assert!(!needs_approval(&read, &none, ApprovalMode::Ask));
        assert!(!needs_approval(&ask_user_tool(), &none, ApprovalMode::Ask));
    }

    #[test]
    fn reading_the_web_needs_no_approval_but_posting_does() {
        let mut fetch = tool(Some("browser"), false);
        fetch.tool_name = "fetch".into();
        assert!(!needs_approval(
            &fetch,
            &json!({"url": "https://x"}),
            ApprovalMode::Ask
        ));
        assert!(!needs_approval(
            &fetch,
            &json!({"method": "get"}),
            ApprovalMode::Ask
        ));
        assert!(needs_approval(
            &fetch,
            &json!({"method": "POST"}),
            ApprovalMode::Ask
        ));
        let mut navigate = tool(Some("browser"), false);
        navigate.tool_name = "navigate".into();
        assert!(!needs_approval(&navigate, &json!({}), ApprovalMode::Ask));
    }

    #[test]
    fn the_registry_offers_ask_user_even_without_connectors() {
        let registry = ToolRegistry::new();
        let tools = available(&registry, None);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, ASK_USER);
        assert!(tools[0].definition().is_some());
    }

    #[test]
    fn a_command_naming_the_app_s_data_always_asks() {
        let shell = tool(Some("bash"), false);
        let dir = std::path::Path::new("/data/cortex");
        let write = json!({ "command": "echo '{}' > /data/cortex/evals/trips/cases/a.json" });
        assert_eq!(
            risk_here(Some(dir), &shell, &write),
            Some(cortex_kernel::approval::PROTECTED)
        );
        assert!(risk_here(Some(dir), &shell, &json!({ "command": "cargo test" })).is_none());
        assert!(risk_here(None, &shell, &write).is_none());
    }
}
