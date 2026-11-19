//! What a tool is to the agent loop: its description as offered to the model, and what a call
//! returned.

use rig_core::completion::message::ToolName;
use rig_core::completion::request::ToolDefinition as RigTool;
use serde_json::{json, Value};

/// The built-in tool the model uses to ask the user a multiple-choice question.
pub const ASK_USER: &str = "ask_user";

/// A tool as offered to the model, and where calls to it go.
#[derive(Debug, Clone)]
pub struct ChatTool {
    /// The name the model sees.
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// `None` for `ask_user`.
    pub connector_id: Option<String>,
    pub tool_name: String,
    pub read_only: bool,
}

impl ChatTool {
    pub fn definition(&self) -> Option<RigTool> {
        Some(RigTool::new(
            ToolName::new(self.name.clone()).ok()?,
            self.description.clone(),
            self.parameters.clone(),
        ))
    }
}

/// Providers accept `^[a-zA-Z0-9_-]{1,64}$`; anything else becomes `_`.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.chars().take(64).collect()
}

/// Every provider wants an object schema with properties, even for a tool without inputs.
pub fn normalize_schema(schema: &Value) -> Value {
    let mut schema = match schema {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let map = schema.as_object_mut().expect("object");
    map.remove("$schema");
    map.entry("type").or_insert_with(|| json!("object"));
    map.entry("properties").or_insert_with(|| json!({}));
    schema
}

/// What a call returned, as text for the model, and whether it failed.
pub struct Outcome {
    pub text: String,
    pub is_error: bool,
}

pub fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}
