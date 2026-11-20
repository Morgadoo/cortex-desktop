//! How far a server's tools are trusted: their default policy, and descriptions cleaned of
//! hidden text, for discovered and cached tools alike.

use super::*;

/// Servers are third-party: only a catalog server's tool that declares itself read-only runs
/// unprompted. A server added by hand can label anything read-only.
pub(super) fn default_policy(read_only: Option<bool>, catalog: bool) -> ToolPolicy {
    if catalog && read_only == Some(true) {
        ToolPolicy::AutoApprove
    } else {
        ToolPolicy::Ask
    }
}

/// Applied to discovered and cached tools alike: caches written by older builds recorded
/// AutoApprove for every tool, and could hold hidden characters.
pub(super) fn vet(mut tool: ToolDefinition, catalog: bool) -> ToolDefinition {
    tool.policy = default_policy(tool.read_only, catalog);
    tool.description = hidden_text::strip(&tool.description);
    tool.title = tool.title.as_deref().map(hidden_text::strip);
    hidden_text::strip_value(&mut tool.input_schema);
    tool
}

pub(super) fn tool_definition(tool: rmcp::model::Tool, catalog: bool) -> ToolDefinition {
    let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
    let definition = ToolDefinition {
        policy: ToolPolicy::Ask,
        input_schema: tool.schema_as_json_value(),
        title: tool
            .title
            .clone()
            .or_else(|| tool.annotations.as_ref().and_then(|a| a.title.clone())),
        read_only,
        open_world: tool.annotations.as_ref().and_then(|a| a.open_world_hint),
        name: tool.name.to_string(),
        description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
    };
    vet(definition, catalog)
}
