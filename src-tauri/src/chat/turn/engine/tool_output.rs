//! Long tool results, kept in files and read back in parts with read_tool_output.

use super::*;

pub const READ_TOOL_OUTPUT: &str = "read_tool_output";

pub(super) fn tool_output_path(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
) -> Option<std::path::PathBuf> {
    let safe = |s: &str| {
        !s.is_empty()
            && s.len() <= 120
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && !s.starts_with('.')
    };
    (safe(conversation) && safe(call_id)).then(|| {
        store::chat_dir(config_dir)
            .join("tool-outputs")
            .join(conversation)
            .join(format!("{call_id}.txt"))
    })
}

/// Keeps a long tool output in `chat/tool-outputs/<conversation>/<call>.txt` (Vibe 3.0.1).
pub(super) fn save_tool_output(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
    text: &str,
) -> Result<(), String> {
    let path = tool_output_path(config_dir, conversation, call_id).ok_or("unusable call id")?;
    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())
}

/// A part of a saved long output, from `offset` characters, at most 20,000.
pub(in crate::chat::turn) fn read_tool_output(
    config_dir: &std::path::Path,
    conversation: &str,
    call_id: &str,
    offset: usize,
) -> Result<String, String> {
    let path = tool_output_path(config_dir, conversation, call_id)
        .ok_or("There is no saved output with that call id.")?;
    let text = crate::chat::at_rest::read_to_string(path).map_err(|_| {
        "There is no saved output with that call id in this conversation.".to_string()
    })?;
    let total = text.chars().count();
    let part: String = text.chars().skip(offset).take(20_000).collect();
    let end = (offset + part.chars().count()).min(total);
    Ok(format!(
        "[characters {offset}–{end} of {total}]\n{part}{}",
        if end < total {
            format!("\n[… continue with offset={end}]")
        } else {
            String::new()
        }
    ))
}

pub(super) fn read_tool_output_tool() -> ChatTool {
    ChatTool {
        name: READ_TOOL_OUTPUT.into(),
        description: "Read a part of a long tool result that was shown only in part: give the call id and the offset the result named.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "call_id": { "type": "string" },
                "offset": { "type": "integer", "minimum": 0 }
            },
            "required": ["call_id"]
        }),
        connector_id: None,
        tool_name: READ_TOOL_OUTPUT.into(),
        read_only: true,
    }
}
