//! Handlers for the context itself: how full the window is, and unloading tools no longer needed.

use super::*;

pub(in crate::chat::turn::engine) const CONTEXT_STATUS: &str = "context_status";
pub(in crate::chat::turn::engine) const UNLOAD_TOOLS: &str = "unload_tools";

pub(in crate::chat::turn::engine) fn context_status_tool() -> ChatTool {
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

pub(in crate::chat::turn::engine) fn unload_tools_tool() -> ChatTool {
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
pub(super) fn on_context_status<'a>(
    r: &'a Runner<'a>,
    call: &'a ToolCall,
    arguments: Value,
    asked: bool,
) -> BoxFuture<'a, tools::Outcome> {
    Box::pin(async move {
        let Ok(c) = store::load_conversation(r.config_dir, r.id) else {
            return tools::Outcome::err("The conversation could not be read.");
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
        tools::Outcome::ok(serde_json::to_string_pretty(&status).unwrap_or_default())
    })
}

/// `unload_tools`: the kernel's `unload(ref)`; the turn drops them from what it sends.
#[allow(unused_variables)]
pub(super) fn on_unload_tools<'a>(
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
        tools::Outcome::ok(serde_json::json!({ "unloaded": names }).to_string())
    })
}
