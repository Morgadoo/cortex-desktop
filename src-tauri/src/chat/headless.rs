//! Headless runs (Vibe's `vibe -p`): `lumo-desktop -p "question"` answers on standard output and
//! exits, with no window, for scripts and other programs.
//!
//! The run uses the model chosen in the app (or `--model vendor/model`), the user's
//! instructions and saved facts, and the built-in tools that only read: web search, libraries,
//! knowledge pages, earlier chats, calendars and contacts. Nothing is saved; nothing asks.
//!
//! ```text
//! lumo-desktop -p "What is on my calendar tomorrow?"
//! echo "Summarize: ..." | lumo-desktop -p - --model xai/deepseek-flash
//! ```

use std::io::Read as _;
use std::path::Path;

use rig_core::completion::message::ToolResultContent;
use rig_core::completion::{CompletionRequest, Message};

use super::builtins;
use super::tools::ChatTool;

/// Tool steps before the answer is asked for.
const MAX_STEPS: usize = 8;

/// The built-in tools a headless run may use: the ones that only read.
const READ_ONLY: &[&str] = &[
    builtins::WEB_SEARCH,
    builtins::SEARCH_LIBRARY,
    builtins::READ_DOCUMENT,
    builtins::KNOWLEDGE_READ,
    builtins::KNOWLEDGE_SEARCH,
    builtins::SEARCH_CHATS,
    builtins::READ_CHAT,
    builtins::CALENDAR_EVENTS,
    builtins::FIND_CONTACT,
];

/// What the command line asked for.
#[derive(Debug, PartialEq)]
pub struct Args {
    pub prompt: String,
    pub model: Option<String>,
    pub tools: bool,
}

/// Reads `-p <prompt>` (or `--print`), `--model <ref>` and `--no-tools`; `None` when there is no
/// `-p`, so the app starts as usual. `-p -`, or `-p` with nothing after it, reads standard input.
pub fn parse(args: &[String], stdin: impl FnOnce() -> String) -> Option<Args> {
    let at = args.iter().position(|a| a == "-p" || a == "--print")?;
    let mut prompt = args.get(at + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "-".into());
    if prompt == "-" {
        prompt = stdin();
    }
    let model = args.iter().position(|a| a == "--model").and_then(|i| args.get(i + 1)).cloned();
    Some(Args { prompt: prompt.trim().to_string(), model, tools: !args.iter().any(|a| a == "--no-tools") })
}

fn read_stdin() -> String {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    text
}

/// Runs the app headless when asked to; returns the exit code, or `None` to start the app.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let parsed = parse(&args, read_stdin)?;
    if parsed.prompt.is_empty() {
        eprintln!("lumo: give a prompt after -p, or pipe one in with -p -");
        return Some(2);
    }
    Some(match tauri::async_runtime::block_on(run(&parsed)) {
        Ok(answer) => {
            println!("{}", answer.trim_end());
            0
        }
        Err(e) => {
            eprintln!("lumo: {e}");
            1
        }
    })
}

/// One request, answered with the read-only tools.
pub async fn run(args: &Args) -> Result<String, String> {
    let config_dir = crate::reporting::early_config_dir().ok_or("the app's config folder could not be found")?;
    let reference = match &args.model {
        Some(m) => m.clone(),
        None => super::providers::load(&config_dir).default_model.ok_or("no model: pass --model vendor/model, or choose one in the app first")?,
    };
    let model = super::engine::connect_model(&config_dir, &reference).await?;
    let tools: Vec<ChatTool> = if args.tools { builtins::definitions(&config_dir).into_iter().filter(|t| READ_ONLY.contains(&t.name.as_str())).collect() } else { Vec::new() };
    let system = preamble(&config_dir);
    let mut messages = vec![Message::user(args.prompt.clone())];
    for step in 0..=MAX_STEPS {
        let last = step == MAX_STEPS;
        let request_messages = super::context::request_messages(&messages, None);
        let Some((prompt, history)) = request_messages.split_last() else { break };
        let mut request = CompletionRequest::new(prompt.clone()).messages(history.iter().cloned()).preamble(if last { format!("{system}\n\nNo more tool calls: answer now from what you found.") } else { system.clone() });
        if !last && !tools.is_empty() {
            request = request.tools(tools.iter().filter_map(ChatTool::definition));
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(300), model.call(request))
            .await
            .map_err(|_| "the model did not answer in time".to_string())?
            .map_err(|e| super::context::short_error(&e.to_string()))?;
        let calls: Vec<_> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            messages.push(message);
        }
        if calls.is_empty() {
            let text = response.text();
            return if text.trim().is_empty() { Err("the model gave no answer".into()) } else { Ok(text) };
        }
        let mut results = Vec::new();
        for call in calls {
            let name = call.function.name.as_str().to_string();
            let outcome = if READ_ONLY.contains(&name.as_str()) {
                builtins::run(&config_dir, &name, &serde_json::Value::Object(call.function.arguments.clone())).await
            } else {
                super::tools::Outcome { text: format!("{name} is not available in a headless run."), is_error: true }
            };
            eprintln!("lumo: used {name}");
            let mut result = call.result(vec![ToolResultContent::text(outcome.text)]);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        messages.push(Message::tool_results(results));
    }
    Err("the run ended without an answer".into())
}

fn preamble(config_dir: &Path) -> String {
    let mut out = String::from(
        "You are Lumo, answering one request from the command line. Nobody can answer questions \
         back, so make reasonable assumptions and state them. Answer in the user's language, \
         plainly, in Markdown only where it helps. When you are unsure, say so.",
    );
    out.push_str(&format!("\n\n{}", super::engine::local_time_line(chrono::Local::now())));
    let instructions = super::store::load_instructions(config_dir);
    if !instructions.name.trim().is_empty() {
        out.push_str(&format!(" The user's name is {}.", instructions.name.trim()));
    }
    if let Some(memory) = super::memory::prompt(&super::memory::load(config_dir)) {
        out.push_str(&memory.replace("you can save lasting facts about the user with `remember` (preferences, their work, people and projects they mention, how they like answers) and remove wrong ones with `forget`. Save only what will help in future conversations, never secrets. ", ""));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line_is_read() {
        assert_eq!(parse(&args(&["lumo"]), String::new), None);
        assert_eq!(parse(&args(&["lumo", "lumo://chat/x"]), String::new), None);
        assert_eq!(
            parse(&args(&["lumo", "-p", "What time is it?", "--model", "xai/deepseek-flash"]), String::new),
            Some(Args { prompt: "What time is it?".into(), model: Some("xai/deepseek-flash".into()), tools: true })
        );
        assert_eq!(parse(&args(&["lumo", "--print", "-", "--no-tools"]), || "from stdin\n".into()), Some(Args { prompt: "from stdin".into(), model: None, tools: false }));
        assert_eq!(parse(&args(&["lumo", "-p"]), || "piped".into()).unwrap().prompt, "piped");
        assert!(READ_ONLY.iter().all(|t| !matches!(*t, builtins::REMEMBER | builtins::FORGET | builtins::SCHEDULE_TASK | builtins::KNOWLEDGE_WRITE)));
    }
}
