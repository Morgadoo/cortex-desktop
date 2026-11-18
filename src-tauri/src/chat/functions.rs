//! Functions (Open WebUI's Functions): code you write that shapes conversations.
//!
//! - A **filter** sees each message before it is sent (`inlet`) and each finished answer
//!   (`outlet`). It can rewrite the text (take personal data out, add context, fix formatting) or
//!   refuse a message (Onyx's query rejection). The script gets `args = {stage, text, model}` and
//!   prints the new text; printing nothing leaves it as it was; printing
//!   `{"block": "why"}` stops the message with that reason.
//! - A **pipe** is a model made of code: it gets `args = {messages, model}` (the conversation as
//!   `{role, content}` pairs) and prints the answer. Pipes appear in the model menu under "Your
//!   functions", as `fn/<id>`; one can call any web API, run a local program or combine models.
//!
//! As with your own tools, the code never runs inside the app: each run uses the code sandbox
//! (isolated with bubblewrap where available). Functions live in `chat/functions.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::store::{chat_dir, read_json, write_json};

pub const VENDOR: &str = "fn";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Function {
    /// Lowercase letters, digits and dashes.
    pub id: String,
    pub name: String,
    /// `filter` or `pipe`.
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// `python`, `javascript` or `bash`.
    pub language: String,
    pub code: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("functions.json")
}

pub fn list(config_dir: &Path) -> Vec<Function> {
    read_json::<Vec<Function>>(&path(config_dir)).unwrap_or_default()
}

pub fn save(config_dir: &Path, f: Function) -> Result<Vec<Function>, String> {
    if f.id.is_empty() || f.id.len() > 48 || !f.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err("Give it an id of lowercase letters, digits and dashes.".into());
    }
    if !matches!(f.kind.as_str(), "filter" | "pipe") {
        return Err("A function is a filter or a pipe.".into());
    }
    if !matches!(f.language.as_str(), "python" | "javascript" | "bash") {
        return Err("Write it in Python, JavaScript or Bash.".into());
    }
    if f.name.trim().is_empty() || f.code.trim().is_empty() {
        return Err("Give it a name and some code.".into());
    }
    let mut all = list(config_dir);
    all.retain(|x| x.id != f.id);
    all.push(f);
    write_json(&path(config_dir), &all)?;
    Ok(all)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Function>, String> {
    let mut all = list(config_dir);
    all.retain(|x| x.id != id);
    write_json(&path(config_dir), &all)?;
    Ok(all)
}

/// Pipes, for the model menu.
pub fn models(config_dir: &Path) -> Vec<super::providers::ModelView> {
    list(config_dir)
        .into_iter()
        .filter(|f| f.enabled && f.kind == "pipe")
        .map(|f| super::providers::ModelView { reference: format!("{VENDOR}/{}", f.id), vendor: VENDOR.into(), provider_label: "Your functions".into(), id: f.id, name: f.name, reasoning: false, tools: false, context_window: None })
        .collect()
}

pub fn find_pipe(config_dir: &Path, reference: &str) -> Option<Function> {
    let id = reference.strip_prefix(VENDOR)?.strip_prefix('/')?;
    list(config_dir).into_iter().find(|f| f.enabled && f.kind == "pipe" && f.id == id)
}

/// Runs a function's code in the sandbox with `args`; returns what it printed.
pub async fn run(config_dir: &Path, conversation_id: &str, f: &Function, args: &Value) -> Result<String, String> {
    let code = super::custom_tools::program_for(&f.language, &f.code, args);
    let outcome = super::sandbox::run(config_dir, conversation_id, &json!({ "language": f.language, "code": code })).await;
    let report: Value = serde_json::from_str(&outcome.text).unwrap_or_else(|_| json!({ "stdout": outcome.text }));
    let stdout = report.get("stdout").and_then(Value::as_str).unwrap_or_default().trim_end().to_string();
    let stderr = report.get("stderr").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if outcome.is_error || report.get("exit_code").and_then(Value::as_i64).is_some_and(|c| c != 0) {
        return Err(format!("{} failed: {}", f.name, if stderr.is_empty() { stdout } else { stderr }));
    }
    Ok(stdout)
}

/// What a filter decided about a text.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Keep,
    Replace(String),
    Block(String),
}

/// A filter's output: nothing keeps the text, `{"block": ...}` refuses it, `{"text": ...}` or
/// plain text replaces it.
pub fn verdict(output: &str) -> Verdict {
    let out = output.trim();
    if out.is_empty() {
        return Verdict::Keep;
    }
    if out.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<Value>(out) {
            if let Some(reason) = v.get("block").and_then(Value::as_str) {
                return Verdict::Block(reason.to_string());
            }
            if let Some(text) = v.get("text").and_then(Value::as_str) {
                return Verdict::Replace(text.to_string());
            }
        }
    }
    Verdict::Replace(output.trim_end().to_string())
}

fn filters(config_dir: &Path) -> Vec<Function> {
    list(config_dir).into_iter().filter(|f| f.enabled && f.kind == "filter").collect()
}

pub fn has_filters(config_dir: &Path) -> bool {
    !filters(config_dir).is_empty()
}

/// Passes a text through every filter in turn. `Err` is the reason a filter refused it. A filter
/// that fails is reported in the log and skipped, so a bug in one does not stop the chat.
pub async fn apply(config_dir: &Path, conversation_id: &str, stage: &str, model: &str, text: &str) -> Result<String, String> {
    let mut text = text.to_string();
    for f in filters(config_dir) {
        match run(config_dir, conversation_id, &f, &json!({ "stage": stage, "text": text, "model": model })).await {
            Ok(out) => match verdict(&out) {
                Verdict::Keep => {}
                Verdict::Replace(t) => text = t,
                Verdict::Block(reason) => return Err(format!("{}: {reason}", f.name)),
            },
            Err(e) => log::warn!("filter {} skipped: {e}", f.id),
        }
    }
    Ok(text)
}

/// The conversation as `{role, content}` pairs, for a pipe.
pub fn messages(conversation: &super::store::Conversation) -> Vec<Value> {
    super::store::turns(conversation)
        .iter()
        .map(|t| {
            let text = t.parts.iter().filter_map(|p| if let super::store::Part::Text { text } = p { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join("\n");
            json!({ "role": t.role, "content": text })
        })
        .filter(|m| !m["content"].as_str().unwrap_or_default().trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: &str, code: &str) -> Function {
        Function { id: "redact".into(), name: "Redact emails".into(), kind: kind.into(), description: String::new(), language: "python".into(), code: code.into(), enabled: true }
    }

    #[test]
    fn functions_are_saved_and_pipes_listed() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), sample("filter", "print(args['text'])")).unwrap();
        assert!(save(dir.path(), Function { id: "Bad Id".into(), ..sample("filter", "x") }).is_err());
        assert!(save(dir.path(), Function { kind: "other".into(), ..sample("filter", "x") }).is_err());
        assert!(models(dir.path()).is_empty() && has_filters(dir.path()));
        save(dir.path(), Function { id: "echo".into(), name: "Echo".into(), ..sample("pipe", "print('hi')") }).unwrap();
        assert_eq!(models(dir.path())[0].reference, "fn/echo");
        assert!(find_pipe(dir.path(), "fn/echo").is_some() && find_pipe(dir.path(), "fn/redact").is_none());
        delete(dir.path(), "echo").unwrap();
        assert!(models(dir.path()).is_empty());
    }

    #[test]
    fn filter_output_becomes_a_verdict() {
        assert_eq!(verdict("  \n"), Verdict::Keep);
        assert_eq!(verdict("Hello [email]"), Verdict::Replace("Hello [email]".into()));
        assert_eq!(verdict(r#"{"block": "No customer numbers here."}"#), Verdict::Block("No customer numbers here.".into()));
        assert_eq!(verdict(r#"{"text": "new"}"#), Verdict::Replace("new".into()));
        // JSON without those keys is just text.
        assert_eq!(verdict(r#"{"a": 1}"#), Verdict::Replace(r#"{"a": 1}"#.into()));
    }
}
