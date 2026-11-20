//! The notebook an agent keeps across its conversations and runs.

use super::*;

/// The tool an agent uses to keep its notebook.
pub const NOTEBOOK_TOOL: &str = "update_notebook";
/// Characters a notebook may hold; it goes into every prompt of the agent.
pub const NOTEBOOK_CHARS: usize = 12_000;

pub(super) fn notebook_path(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("invalid agent id".into());
    }
    Ok(chat_dir(config_dir)
        .join("agents")
        .join(id)
        .join("notebook.md"))
}

/// The agent's notebook: what it keeps across conversations and runs, in Markdown.
pub fn notebook(config_dir: &Path, id: &str) -> String {
    notebook_path(config_dir, id)
        .ok()
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn save_notebook(config_dir: &Path, id: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.chars().count() > NOTEBOOK_CHARS {
        return Err(format!(
            "The notebook holds at most {NOTEBOOK_CHARS} characters; condense it."
        ));
    }
    let path = notebook_path(config_dir, id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The notebook as it was stays restorable.
    crate::chat::versions::keep(
        config_dir,
        "notebooks",
        id,
        &notebook(config_dir, id),
        &text.to_string(),
    )?;
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())
}

pub fn notebook_tool() -> cortex_kernel::tool::ChatTool {
    cortex_kernel::tool::ChatTool {
        name: NOTEBOOK_TOOL.into(),
        description: format!(
            "Rewrite your notebook: the working memory you keep across all your conversations and \
             scheduled runs. Keep it current and short (at most {NOTEBOOK_CHARS} characters): what \
             you learned about the work, decisions, preferences, open threads, and what to do \
             differently next time. Send the whole notebook, not a diff."
        ),
        parameters: serde_json::json!({ "type": "object", "properties": { "content": { "type": "string", "description": "The full notebook, in Markdown." } }, "required": ["content"] }),
        connector_id: None,
        tool_name: NOTEBOOK_TOOL.into(),
        read_only: true,
    }
}

/// The part of the agent's prompt about its notebook.
pub fn notebook_prompt(text: &str) -> String {
    let body = if text.trim().is_empty() {
        "(empty: start it when you learn something worth keeping)"
    } else {
        text.trim()
    };
    format!(
        "\n\n## Your notebook\nYou keep this across every conversation and run; the user can read and \
         edit it. Update it with `{NOTEBOOK_TOOL}` when you learn something that will matter next \
         time, before you finish.\n<notebook>\n{body}\n</notebook>"
    )
}
