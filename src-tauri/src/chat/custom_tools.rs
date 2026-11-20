//! Tools you write yourself, as Open WebUI's Tools: a script with a JSON schema for its input.
//!
//! Unlike Open WebUI, the code never runs inside the app: each call runs the script in the same
//! sandbox as `run_code` (isolated with bubblewrap where available), with the call's arguments in
//! the variable `args`, and returns what it prints. Every call passes the approval gate like a
//! connector tool. Tools live in `chat/tools/<name>/tool.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::store::{chat_dir, write_json};
use crate::chat::tools::ChatTool;

pub const CONNECTOR: &str = "custom";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomTool {
    /// Lowercase letters, digits and underscores.
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments (an object schema).
    pub parameters: Value,
    /// `python`, `javascript` or `bash`.
    pub language: String,
    pub code: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn dir(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("tools")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with('_')
}

pub fn list(config_dir: &Path) -> Vec<CustomTool> {
    let mut out: Vec<CustomTool> = std::fs::read_dir(dir(config_dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            serde_json::from_str(
                &crate::chat::at_rest::read_to_string(e.path().join("tool.json")).ok()?,
            )
            .ok()
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn save(config_dir: &Path, tool: &CustomTool) -> Result<Vec<CustomTool>, String> {
    if !valid_name(&tool.name) {
        return Err(
            "A tool name uses lowercase letters, digits and underscores, like `convert_units`."
                .into(),
        );
    }
    if tool.description.trim().is_empty() {
        return Err(
            "Describe what the tool does; the model reads it to decide when to call it.".into(),
        );
    }
    if !["python", "javascript", "bash"].contains(&tool.language.as_str()) {
        return Err("Choose Python, JavaScript or Bash.".into());
    }
    if tool.parameters.get("type").and_then(Value::as_str) != Some("object") {
        return Err("The input schema must be a JSON object schema, like {\"type\": \"object\", \"properties\": {…}}.".into());
    }
    if let Some(previous) = list(config_dir).into_iter().find(|t| t.name == tool.name) {
        crate::chat::versions::keep(config_dir, "tools", &tool.name, &previous, tool)?;
    }
    write_json(&dir(config_dir).join(&tool.name).join("tool.json"), tool)?;
    Ok(list(config_dir))
}

pub fn delete(config_dir: &Path, name: &str) -> Result<Vec<CustomTool>, String> {
    if !valid_name(name) {
        return Err("invalid tool name".into());
    }
    let _ = std::fs::remove_dir_all(dir(config_dir).join(name));
    Ok(list(config_dir))
}

/// The enabled tools, as the model sees them.
pub fn definitions(config_dir: &Path) -> Vec<ChatTool> {
    list(config_dir)
        .into_iter()
        .filter(|t| t.enabled)
        .map(|t| ChatTool {
            name: format!("tool__{}", t.name),
            description: t.description,
            parameters: t.parameters,
            connector_id: Some(CONNECTOR.into()),
            tool_name: t.name,
            read_only: false,
        })
        .collect()
}

/// The script with the call's arguments bound to `args` (`ARGS`, as JSON text, in Bash).
pub fn program(tool: &CustomTool, args: &Value) -> String {
    program_for(&tool.language, &tool.code, args)
}

/// `code` with `args` bound, in its language (also used by functions).
pub fn program_for(language: &str, code: &str, args: &Value) -> String {
    let json = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
    match language {
        // A JSON string literal is a valid Python string literal for `json.loads`.
        "python" => format!(
            "import json as __json\nargs = __json.loads({})\n{}",
            serde_json::to_string(&json).unwrap_or_default(),
            code
        ),
        "javascript" => format!("const args = {json};\n{code}"),
        _ => format!("ARGS='{}'\n{code}", json.replace('\'', "'\\''")),
    }
}

/// A JavaScript tool that uses nothing outside the language (no Node modules, process or files).
pub fn runs_in_wasm(tool: &CustomTool) -> bool {
    tool.language == "javascript"
        && ![
            "require(",
            "import ",
            "import(",
            "process.",
            "__dirname",
            "Deno.",
            "Bun.",
            "fetch(",
        ]
        .iter()
        .any(|needle| tool.code.contains(needle))
}

/// Runs a tool in the sandbox; returns what it printed, or why it failed.
pub async fn run(
    config_dir: &Path,
    conversation_id: &str,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    let tool = list(config_dir)
        .into_iter()
        .find(|t| t.name == name)
        .ok_or("That tool was removed.")?;
    // JavaScript that needs nothing from Node runs in the WebAssembly sandbox, which works the same
    // on every system; what it prints is its result. Anything else keeps the process sandbox.
    if runs_in_wasm(&tool) {
        crate::chat::script_tools::sandbox::cache_in(config_dir);
        let (result, _) = crate::chat::script_tools::sandbox::run(
            &program(&tool, args),
            "{}",
            &Default::default(),
            Default::default(),
            None,
        )
        .await;
        return result
            .map(|out| out.trim().to_string())
            .map_err(|e| format!("The tool failed: {e}"));
    }
    let outcome = crate::chat::sandbox::run(
        config_dir,
        conversation_id,
        &json!({ "language": tool.language, "code": program(&tool, args) }),
    )
    .await;
    let report: Value =
        serde_json::from_str(&outcome.text).unwrap_or_else(|_| json!({ "stdout": outcome.text }));
    let stdout = report
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let stderr = report
        .get("stderr")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let failed = outcome.is_error
        || report
            .get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|c| c != 0);
    if failed {
        Err(if stderr.is_empty() {
            format!("The tool failed. {stdout}")
        } else {
            format!("The tool failed: {stderr}")
        })
    } else {
        Ok(if stdout.is_empty() {
            "(The tool printed nothing.)".into()
        } else {
            stdout
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(language: &str, code: &str) -> CustomTool {
        CustomTool {
            name: "convert_units".into(),
            description: "Convert kilometres to miles.".into(),
            parameters: json!({ "type": "object", "properties": { "km": { "type": "number" } }, "required": ["km"] }),
            language: language.into(),
            code: code.into(),
            enabled: true,
        }
    }

    #[test]
    fn tools_are_saved_and_offered() {
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            &sample("python", "print(args['km'] * 0.621371)"),
        )
        .unwrap();
        assert!(save(
            dir.path(),
            &CustomTool {
                name: "Bad Name".into(),
                ..sample("python", "")
            }
        )
        .is_err());
        assert!(save(
            dir.path(),
            &CustomTool {
                parameters: json!({ "type": "string" }),
                ..sample("python", "")
            }
        )
        .is_err());
        let defs = definitions(dir.path());
        assert_eq!(defs.len(), 1);
        assert_eq!(
            (defs[0].name.as_str(), defs[0].connector_id.as_deref()),
            ("tool__convert_units", Some(CONNECTOR))
        );
        delete(dir.path(), "convert_units").unwrap();
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn arguments_reach_the_script_safely() {
        let args = json!({ "km": 10, "note": "it's \"quoted\"\nline" });
        let py = program(&sample("python", "print(args)"), &args);
        assert!(py.starts_with("import json as __json\nargs = __json.loads(\""));
        let sh = program(&sample("bash", "echo \"$ARGS\""), &args);
        assert!(sh.contains("it'\\''s"), "{sh}");
        assert!(
            program(&sample("javascript", "console.log(args.km)"), &args)
                .starts_with("const args = {")
        );
    }

    #[tokio::test]
    async fn a_python_tool_runs_in_the_sandbox() {
        if crate::chat::sandbox::languages()
            .iter()
            .all(|l| *l != "python")
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            &sample("python", "print(round(args['km'] * 0.621371, 2))"),
        )
        .unwrap();
        assert_eq!(
            run(dir.path(), "c1", "convert_units", &json!({ "km": 10 }))
                .await
                .unwrap(),
            "6.21"
        );
        save(
            dir.path(),
            &sample("python", "raise SystemExit('bad input')"),
        )
        .unwrap();
        assert!(run(dir.path(), "c1", "convert_units", &json!({}))
            .await
            .unwrap_err()
            .contains("bad input"));
    }
}

#[cfg(test)]
mod wasm_tests {
    use super::*;

    #[tokio::test]
    async fn plain_javascript_tools_run_in_the_wasm_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let tool = CustomTool {
            name: "shout".into(),
            description: "Upper-cases a text.".into(),
            parameters: json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
            language: "javascript".into(),
            code: "console.log(args.text.toUpperCase());".into(),
            enabled: true,
        };
        assert!(runs_in_wasm(&tool));
        save(dir.path(), &tool).unwrap();
        assert_eq!(
            run(dir.path(), "conv", "shout", &json!({ "text": "hi" }))
                .await
                .unwrap(),
            "HI"
        );
        let node = CustomTool {
            code: "const fs = require('fs'); console.log(1);".into(),
            ..tool.clone()
        };
        assert!(!runs_in_wasm(&node));
        assert!(!runs_in_wasm(&CustomTool {
            language: "python".into(),
            ..tool
        }));
    }
}
