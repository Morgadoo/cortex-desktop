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
//! lumo-desktop -p --prompt-file task.md --max-turns 4 --time-limit 120 --output-dir out/
//! ```
//!
//! Limits, as Vibe's: `--max-turns` (tool steps), `--max-tokens` (input and output together),
//! `--max-price` (US dollars, where the provider reports cost) and `--time-limit` (seconds).
//! `--output-dir` writes `export.json`: the prompt, model, answer, every tool step, usage, cost,
//! time and how the run ended. Exit codes: 0 done, 1 a usage or settings error, 2 the model or
//! network failed, 3 a limit was reached or the model refused.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rig_core::completion::message::ToolResultContent;
use rig_core::completion::{CompletionRequest, Message};
use serde::Serialize;

use super::builtins;
use super::tools::ChatTool;

/// Tool steps before the answer is asked for, unless `--max-turns` says otherwise.
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
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub prompt: String,
    pub model: Option<String>,
    pub tools: bool,
    pub max_turns: Option<usize>,
    pub max_tokens: Option<u64>,
    pub max_price: Option<f64>,
    pub time_limit: Option<u64>,
    pub output_dir: Option<PathBuf>,
}

/// How a run ended, with its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    Done,
    UsageError,
    ModelError,
    Limit,
}

impl Ending {
    pub fn code(self) -> i32 {
        match self {
            Ending::Done => 0,
            Ending::UsageError => 1,
            Ending::ModelError => 2,
            Ending::Limit => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub tool: String,
    pub input: serde_json::Value,
    pub output: String,
    pub error: bool,
}

/// What `--output-dir` writes as `export.json`.
#[derive(Debug, Clone, Serialize)]
pub struct Export {
    pub prompt: String,
    pub model: String,
    pub answer: String,
    pub ending: Ending,
    pub exit_code: i32,
    pub message: String,
    pub steps: Vec<Step>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub seconds: f64,
}

fn value_of(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// Reads `-p <prompt>` (or `--print`) and its options; `None` when there is no `-p`, so the app
/// starts as usual. `-p -`, or `-p` with nothing after it, reads standard input; `--prompt-file`
/// reads a file. A bad option is an `Err` (exit code 1).
pub fn parse(args: &[String], stdin: impl FnOnce() -> String) -> Option<Result<Args, String>> {
    let at = args.iter().position(|a| a == "-p" || a == "--print")?;
    Some((|| {
        let number = |flag: &str| -> Result<Option<f64>, String> {
            value_of(args, flag).map(|v| v.parse::<f64>().ok().filter(|n| *n > 0.0).ok_or(format!("{flag} takes a positive number, not `{v}`"))).transpose()
        };
        let mut prompt = match value_of(args, "--prompt-file") {
            Some(path) => crate::chat::at_rest::read_to_string(&path).map_err(|e| format!("--prompt-file {path}: {e}"))?,
            None => args.get(at + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "-".into()),
        };
        if prompt == "-" {
            prompt = stdin();
        }
        Ok(Args {
            prompt: prompt.trim().to_string(),
            model: value_of(args, "--model"),
            tools: !args.iter().any(|a| a == "--no-tools"),
            max_turns: number("--max-turns")?.map(|n| n as usize),
            max_tokens: number("--max-tokens")?.map(|n| n as u64),
            max_price: number("--max-price")?,
            time_limit: number("--time-limit")?.map(|n| n as u64),
            output_dir: value_of(args, "--output-dir").map(PathBuf::from),
        })
    })())
}

fn read_stdin() -> String {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    text
}

/// Runs the app headless when asked to; returns the exit code, or `None` to start the app.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let parsed = match parse(&args, read_stdin)? {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lumo: {e}");
            return Some(1);
        }
    };
    if parsed.prompt.is_empty() {
        eprintln!("lumo: give a prompt after -p, with --prompt-file, or pipe one in with -p -");
        return Some(1);
    }
    let export = tauri::async_runtime::block_on(run(&parsed));
    if let Some(dir) = &parsed.output_dir {
        if let Err(e) = write_export(dir, &export) {
            eprintln!("lumo: could not write the export: {e}");
        }
    }
    if !export.answer.trim().is_empty() {
        println!("{}", export.answer.trim_end());
    }
    if export.ending != Ending::Done {
        eprintln!("lumo: {}", export.message);
    }
    Some(export.exit_code)
}

pub fn write_export(dir: &Path, export: &Export) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(dir.join("export.json"), serde_json::to_string_pretty(export).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// One request, answered with the read-only tools, within the limits asked for.
pub async fn run(args: &Args) -> Export {
    let started = Instant::now();
    let mut export = Export { prompt: args.prompt.clone(), model: String::new(), answer: String::new(), ending: Ending::Done, exit_code: 0, message: String::new(), steps: Vec::new(), input_tokens: 0, output_tokens: 0, cost: 0.0, seconds: 0.0 };
    let end = |mut e: Export, ending: Ending, message: String| {
        e.ending = ending;
        e.exit_code = ending.code();
        e.message = message;
        e.seconds = started.elapsed().as_secs_f64();
        e
    };
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        return end(export, Ending::UsageError, "the app's config folder could not be found".into());
    };
    let reference = match &args.model {
        Some(m) => m.clone(),
        None => match super::providers::load(&config_dir).default_model {
            Some(m) => m,
            None => return end(export, Ending::UsageError, "no model: pass --model vendor/model, or choose one in the app first".into()),
        },
    };
    export.model = reference.clone();
    let model = match super::engine::connect_model(&config_dir, &reference).await {
        Ok(m) => m,
        Err(e) => return end(export, Ending::UsageError, e),
    };
    let tools: Vec<ChatTool> = if args.tools { builtins::definitions(&config_dir).into_iter().filter(|t| READ_ONLY.contains(&t.name.as_str())).collect() } else { Vec::new() };
    let system = preamble(&config_dir);
    let max_steps = args.max_turns.unwrap_or(MAX_STEPS);
    let deadline = args.time_limit.map(|s| started + Duration::from_secs(s));
    let mut messages = vec![Message::user(args.prompt.clone())];
    for step in 0..=max_steps {
        let last = step == max_steps;
        let request_messages = super::context::request_messages(&messages, None);
        let Some((prompt, history)) = request_messages.split_last() else { break };
        let mut request = CompletionRequest::new(prompt.clone()).messages(history.iter().cloned()).preamble(if last { format!("{system}\n\nNo more tool calls: answer now from what you found.") } else { system.clone() });
        if !last && !tools.is_empty() {
            request = request.tools(tools.iter().filter_map(ChatTool::definition));
        }
        // Dropped connections and overloaded servers are retried, as in the app.
        let mut attempt = 0u32;
        let response = loop {
            attempt += 1;
            let wait = deadline.map_or(Duration::from_secs(300), |d| d.saturating_duration_since(Instant::now()).min(Duration::from_secs(300)));
            match tokio::time::timeout(wait, model.call(request.clone())).await {
                Err(_) if deadline.is_some_and(|d| Instant::now() >= d) => return end(export, Ending::Limit, format!("the time limit of {} s was reached", args.time_limit.unwrap_or_default())),
                Err(_) => return end(export, Ending::ModelError, "the model did not answer in time".into()),
                Ok(Err(e)) if e.is_retryable() && attempt < 4 && deadline.is_none_or(|d| Instant::now() < d) => {
                    eprintln!("lumo: retrying ({})", super::context::brief_error(&e.to_string()));
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                }
                Ok(Err(e)) => return end(export, Ending::ModelError, super::context::short_error(&e.to_string())),
                Ok(Ok(r)) => break r,
            }
        };
        export.input_tokens += response.usage.input_tokens.unwrap_or(0);
        export.output_tokens += response.usage.output_tokens.unwrap_or(0);
        export.cost += response.usage.cost.map_or(0.0, |c| c.total);
        let calls: Vec<_> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            messages.push(message);
        }
        if calls.is_empty() {
            let text = response.text();
            export.answer = text.clone();
            if text.trim().is_empty() {
                return end(export, Ending::Limit, "the model gave no answer (it may have refused)".into());
            }
            return end(export, Ending::Done, String::new());
        }
        if let Some(limit) = args.max_tokens.filter(|l| export.input_tokens + export.output_tokens > *l) {
            return end(export, Ending::Limit, format!("the token limit of {limit} was reached"));
        }
        if let Some(limit) = args.max_price.filter(|l| export.cost > *l) {
            return end(export, Ending::Limit, format!("the price limit of ${limit} was reached"));
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return end(export, Ending::Limit, format!("the time limit of {} s was reached", args.time_limit.unwrap_or_default()));
        }
        let mut results = Vec::new();
        for call in calls {
            let name = call.function.name.as_str().to_string();
            let input = serde_json::Value::Object(call.function.arguments.clone());
            let outcome = if READ_ONLY.contains(&name.as_str()) {
                builtins::run(&config_dir, &name, &input).await
            } else {
                super::tools::Outcome { text: format!("{name} is not available in a headless run."), is_error: true }
            };
            eprintln!("lumo: used {name}");
            export.steps.push(Step { tool: name, input, output: outcome.text.chars().take(4_000).collect(), error: outcome.is_error });
            let mut result = call.result(vec![ToolResultContent::text(outcome.text)]);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        messages.push(Message::tool_results(results));
    }
    end(export, Ending::Limit, format!("the run ended without an answer after {max_steps} tool steps"))
}

pub fn preamble(config_dir: &Path) -> String {
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
        let a = parse(&args(&["lumo", "-p", "What time is it?", "--model", "xai/deepseek-flash"]), String::new).unwrap().unwrap();
        assert_eq!((a.prompt.as_str(), a.model.as_deref(), a.tools), ("What time is it?", Some("xai/deepseek-flash"), true));
        let a = parse(&args(&["lumo", "--print", "-", "--no-tools"]), || "from stdin\n".into()).unwrap().unwrap();
        assert_eq!((a.prompt.as_str(), a.tools), ("from stdin", false));
        assert_eq!(parse(&args(&["lumo", "-p"]), || "piped".into()).unwrap().unwrap().prompt, "piped");
        assert!(READ_ONLY.iter().all(|t| !matches!(*t, builtins::REMEMBER | builtins::FORGET | builtins::SCHEDULE_TASK | builtins::KNOWLEDGE_WRITE)));
    }

    #[test]
    fn limits_and_files_are_read_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("task.md");
        crate::chat::at_rest::write(&file, "  Summarize the report.\n").unwrap();
        let a = parse(&args(&["lumo", "-p", "--prompt-file", file.to_str().unwrap(), "--max-turns", "3", "--max-tokens", "5000", "--max-price", "0.25", "--time-limit", "60", "--output-dir", "out"]), String::new).unwrap().unwrap();
        assert_eq!(a.prompt, "Summarize the report.");
        assert_eq!((a.max_turns, a.max_tokens, a.max_price, a.time_limit), (Some(3), Some(5000), Some(0.25), Some(60)));
        assert_eq!(a.output_dir, Some(PathBuf::from("out")));
        assert!(parse(&args(&["lumo", "-p", "x", "--max-turns", "lots"]), String::new).unwrap().is_err());
        assert!(parse(&args(&["lumo", "-p", "--prompt-file", "/no/such/file"]), String::new).unwrap().is_err());
        assert_eq!([Ending::Done, Ending::UsageError, Ending::ModelError, Ending::Limit].map(Ending::code), [0, 1, 2, 3]);
    }

    #[test]
    fn the_export_is_written_as_json() {
        let dir = tempfile::tempdir().unwrap();
        let export = Export { prompt: "p".into(), model: "m".into(), answer: "a".into(), ending: Ending::Limit, exit_code: 3, message: "the time limit of 5 s was reached".into(), steps: vec![Step { tool: "web_search".into(), input: serde_json::json!({"query": "q"}), output: "r".into(), error: false }], input_tokens: 10, output_tokens: 2, cost: 0.001, seconds: 1.5 };
        write_export(&dir.path().join("out"), &export).unwrap();
        let v: serde_json::Value = serde_json::from_str(&crate::chat::at_rest::read_to_string(dir.path().join("out/export.json")).unwrap()).unwrap();
        assert_eq!((v["ending"].as_str(), v["exit_code"].as_i64(), v["steps"][0]["tool"].as_str()), (Some("limit"), Some(3), Some("web_search")));
    }
}
