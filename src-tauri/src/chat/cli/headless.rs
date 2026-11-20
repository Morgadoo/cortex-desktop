//! Headless runs (Vibe's `vibe -p`): `cortex-desktop -p "question"` answers on standard output and
//! exits, with no window, for scripts and other programs.
//!
//! The run uses the model chosen in the app (or `--model vendor/model`), the user's
//! instructions and saved facts, and the built-in tools that only read: web search, libraries,
//! knowledge pages, earlier chats, calendars and contacts. Nothing is saved; nothing asks.
//!
//! ```text
//! cortex-desktop -p "What is on my calendar tomorrow?"
//! echo "Summarize: ..." | cortex-desktop -p - --model xai/deepseek-flash
//! cortex-desktop -p --prompt-file task.md --max-turns 4 --time-limit 120 --output-dir out/
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

use std::sync::{Arc, Mutex};

use rig_core::completion::Message;
use serde::Serialize;

use crate::chat::builtins;
use crate::chat::engine::{ChatEvent, TurnRequest};
use crate::chat::headless_host::HeadlessHost;
use crate::chat::host::Host;
use crate::chat::tools::ApprovalMode;

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
    pub(crate) fn code(self) -> i32 {
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
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Reads `-p <prompt>` (or `--print`) and its options; `None` when there is no `-p`, so the app
/// starts as usual. `-p -`, or `-p` with nothing after it, reads standard input; `--prompt-file`
/// reads a file. A bad option is an `Err` (exit code 1).
pub(in crate::chat::cli) fn parse(
    args: &[String],
    stdin: impl FnOnce() -> String,
) -> Option<Result<Args, String>> {
    let at = args.iter().position(|a| a == "-p" || a == "--print")?;
    Some((|| {
        let number = |flag: &str| -> Result<Option<f64>, String> {
            value_of(args, flag)
                .map(|v| {
                    v.parse::<f64>()
                        .ok()
                        .filter(|n| *n > 0.0)
                        .ok_or(format!("{flag} takes a positive number, not `{v}`"))
                })
                .transpose()
        };
        let mut prompt = match value_of(args, "--prompt-file") {
            Some(path) => crate::chat::at_rest::read_to_string(&path)
                .map_err(|e| format!("--prompt-file {path}: {e}"))?,
            None => args
                .get(at + 1)
                .filter(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "-".into()),
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
pub(crate) fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let parsed = match parse(&args, read_stdin)? {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cortex: {e}");
            return Some(1);
        }
    };
    if parsed.prompt.is_empty() {
        eprintln!("cortex: give a prompt after -p, with --prompt-file, or pipe one in with -p -");
        return Some(1);
    }
    let export = tauri::async_runtime::block_on(run(&parsed));
    if let Some(dir) = &parsed.output_dir {
        if let Err(e) = write_export(dir, &export) {
            eprintln!("cortex: could not write the export: {e}");
        }
    }
    if !export.answer.trim().is_empty() {
        println!("{}", export.answer.trim_end());
    }
    if export.ending != Ending::Done {
        eprintln!("cortex: {}", export.message);
    }
    Some(export.exit_code)
}

pub(in crate::chat::cli) fn write_export(dir: &Path, export: &Export) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::chat::at_rest::write(
        dir.join("export.json"),
        serde_json::to_string_pretty(export).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// The tool steps of a turn, from its events.
fn steps_of(events: &[ChatEvent]) -> Vec<Step> {
    let mut steps: Vec<(String, Step)> = Vec::new();
    for event in events {
        match event {
            ChatEvent::ToolStart {
                call_id,
                name,
                arguments,
                ..
            } => steps.push((
                call_id.clone(),
                Step {
                    tool: name.clone(),
                    input: arguments.clone(),
                    output: String::new(),
                    error: false,
                },
            )),
            ChatEvent::ToolEnd {
                call_id,
                output,
                is_error,
                ..
            } => {
                if let Some((_, step)) = steps.iter_mut().rev().find(|(id, _)| id == call_id) {
                    step.output = crate::support::text::clip(output, 4_000);
                    step.error = *is_error;
                }
            }
            _ => {}
        }
    }
    steps.into_iter().map(|(_, step)| step).collect()
}

/// One request, answered with the read-only tools, within the limits asked for. It runs through
/// the app's own agent loop, unattended, in a temporary conversation that is deleted after.
pub(in crate::chat::cli) async fn run(args: &Args) -> Export {
    let started = Instant::now();
    let mut export = Export {
        prompt: args.prompt.clone(),
        model: String::new(),
        answer: String::new(),
        ending: Ending::Done,
        exit_code: 0,
        message: String::new(),
        steps: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cost: 0.0,
        seconds: 0.0,
    };
    let end = |mut e: Export, ending: Ending, message: String| {
        e.ending = ending;
        e.exit_code = ending.code();
        e.message = message;
        e.seconds = started.elapsed().as_secs_f64();
        e
    };
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        return end(
            export,
            Ending::UsageError,
            "the app's config folder could not be found".into(),
        );
    };
    let reference = match &args.model {
        Some(m) => m.clone(),
        None => match crate::chat::providers::load(&config_dir).default_model {
            Some(m) => m,
            None => {
                return end(
                    export,
                    Ending::UsageError,
                    "no model: pass --model vendor/model, or choose one in the app first".into(),
                )
            }
        },
    };
    export.model = reference.clone();
    // A model that cannot be reached at all is a settings problem, not a failed run.
    if let Err(e) = crate::chat::engine::connect_model(&config_dir, &reference).await {
        return end(export, Ending::UsageError, e.to_string());
    }
    let id = format!("headless-{}", uuid::Uuid::new_v4().simple());
    // Named already, so the engine does not write a title after the run is gone.
    let mut conversation =
        crate::chat::store::Conversation::new(id.clone(), None, Some(reference.clone()));
    conversation.temporary = true;
    conversation.title = "Headless run".into();
    if let Err(e) = crate::chat::store::save_conversation(&config_dir, &conversation) {
        return end(export, Ending::UsageError, e.to_string());
    }
    let host = Arc::new(HeadlessHost {
        config_dir: config_dir.clone(),
        conversation: id.clone(),
        state: crate::chat::ChatState::default(),
        registry: crate::tools::registry::ToolRegistry::new(),
        events: Mutex::new(Vec::new()),
        max_tokens: args.max_tokens,
        max_price: args.max_price,
        stopped: Mutex::new(None),
    });
    let request = TurnRequest {
        conversation_id: id.clone(),
        text: args.prompt.clone(),
        model: reference,
        approval: ApprovalMode::Unattended,
        temporary: true,
        connectors: Some(Vec::new()),
        // The last step answers, so `--max-turns` tool steps leave one more for the answer.
        max_steps: Some(args.max_turns.unwrap_or(MAX_STEPS) + 1),
        only_tools: Some(if args.tools {
            READ_ONLY.iter().map(|t| t.to_string()).collect()
        } else {
            Vec::new()
        }),
        ..TurnRequest::default()
    };
    let cancel = host.state.begin(&id).expect("a new conversation");
    if let Some(seconds) = args.time_limit {
        let host = host.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            host.stop(format!("the time limit of {seconds} s was reached"));
        });
    }
    let dyn_host: Arc<dyn Host> = host.clone();
    let outcome = crate::chat::engine::turn(&dyn_host, &request, cancel).await;
    host.state.finish(&id);

    let events = host
        .events
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    export.steps = steps_of(&events);
    if let Some(usage) = events.iter().rev().find_map(|e| match e {
        ChatEvent::Usage { usage, .. } => Some(usage.clone()),
        _ => None,
    }) {
        export.input_tokens = usage.input_tokens;
        export.output_tokens = usage.output_tokens;
        export.cost = usage.cost;
    }
    let answer = crate::chat::store::load_conversation(&config_dir, &id)
        .ok()
        .and_then(|c| {
            c.messages
                .last()
                .filter(|m| matches!(m, Message::Assistant(_)))
                .map(crate::chat::context::message_text)
        })
        .unwrap_or_default();
    // Nothing is kept: a temporary conversation is deleted outright.
    let _ = crate::chat::store::delete_conversation(&config_dir, &id);
    let stopped = host
        .stopped
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    match outcome {
        Err(e) => end(
            export,
            Ending::ModelError,
            crate::chat::context::short_error(&e.to_string()),
        ),
        Ok(true) => end(
            export,
            Ending::Limit,
            stopped.unwrap_or_else(|| "the run was stopped".into()),
        ),
        Ok(false) if answer.trim().is_empty() => end(
            export,
            Ending::Limit,
            "the model gave no answer (it may have refused)".into(),
        ),
        Ok(false) => {
            export.answer = answer;
            end(export, Ending::Done, String::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A limit reached mid-run stops the turn once, with the first reason.
    #[test]
    fn a_limit_cancels_the_running_turn() {
        let host = HeadlessHost {
            config_dir: PathBuf::new(),
            conversation: "h".into(),
            state: crate::chat::ChatState::default(),
            registry: crate::tools::registry::ToolRegistry::new(),
            events: Mutex::new(Vec::new()),
            max_tokens: Some(100),
            max_price: Some(0.5),
            stopped: Mutex::new(None),
        };
        let mut cancel = host.state.begin("h").unwrap();
        let usage = |tokens: u64, cost: f64| ChatEvent::Usage {
            conversation_id: "h".into(),
            usage: crate::chat::context::UsageTotals {
                input_tokens: tokens,
                cost,
                ..Default::default()
            },
            context_window: 0,
        };
        host.emit(usage(50, 0.1));
        assert!(!*cancel.borrow_and_update());
        host.emit(usage(150, 0.9));
        assert!(*cancel.borrow_and_update());
        assert_eq!(
            host.stopped.lock().unwrap().as_deref(),
            Some("the token limit of 100 was reached")
        );
    }

    #[test]
    fn the_export_lists_each_tool_step_with_its_result() {
        let events = vec![
            ChatEvent::ToolStart {
                conversation_id: "h".into(),
                call_id: "1".into(),
                name: "web_search".into(),
                arguments: serde_json::json!({"query": "rust"}),
            },
            ChatEvent::ToolEnd {
                conversation_id: "h".into(),
                call_id: "1".into(),
                output: "results".into(),
                is_error: false,
            },
        ];
        let steps = steps_of(&events);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "web_search");
        assert_eq!(steps[0].output, "results");
        assert!(!steps[0].error);
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line_is_read() {
        assert_eq!(parse(&args(&["cortex"]), String::new), None);
        assert_eq!(
            parse(&args(&["cortex", "cortex://chat/x"]), String::new),
            None
        );
        let a = parse(
            &args(&[
                "cortex",
                "-p",
                "What time is it?",
                "--model",
                "xai/deepseek-flash",
            ]),
            String::new,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (a.prompt.as_str(), a.model.as_deref(), a.tools),
            ("What time is it?", Some("xai/deepseek-flash"), true)
        );
        let a = parse(&args(&["cortex", "--print", "-", "--no-tools"]), || {
            "from stdin\n".into()
        })
        .unwrap()
        .unwrap();
        assert_eq!((a.prompt.as_str(), a.tools), ("from stdin", false));
        assert_eq!(
            parse(&args(&["cortex", "-p"]), || "piped".into())
                .unwrap()
                .unwrap()
                .prompt,
            "piped"
        );
        assert!(READ_ONLY.iter().all(|t| !matches!(
            *t,
            builtins::REMEMBER
                | builtins::FORGET
                | builtins::SCHEDULE_TASK
                | builtins::KNOWLEDGE_WRITE
        )));
    }

    #[test]
    fn limits_and_files_are_read_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("task.md");
        crate::chat::at_rest::write(&file, "  Summarize the report.\n").unwrap();
        let a = parse(
            &args(&[
                "cortex",
                "-p",
                "--prompt-file",
                file.to_str().unwrap(),
                "--max-turns",
                "3",
                "--max-tokens",
                "5000",
                "--max-price",
                "0.25",
                "--time-limit",
                "60",
                "--output-dir",
                "out",
            ]),
            String::new,
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.prompt, "Summarize the report.");
        assert_eq!(
            (a.max_turns, a.max_tokens, a.max_price, a.time_limit),
            (Some(3), Some(5000), Some(0.25), Some(60))
        );
        assert_eq!(a.output_dir, Some(PathBuf::from("out")));
        assert!(parse(
            &args(&["cortex", "-p", "x", "--max-turns", "lots"]),
            String::new
        )
        .unwrap()
        .is_err());
        assert!(parse(
            &args(&["cortex", "-p", "--prompt-file", "/no/such/file"]),
            String::new
        )
        .unwrap()
        .is_err());
        assert_eq!(
            [
                Ending::Done,
                Ending::UsageError,
                Ending::ModelError,
                Ending::Limit
            ]
            .map(Ending::code),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn the_export_is_written_as_json() {
        let dir = tempfile::tempdir().unwrap();
        let export = Export {
            prompt: "p".into(),
            model: "m".into(),
            answer: "a".into(),
            ending: Ending::Limit,
            exit_code: 3,
            message: "the time limit of 5 s was reached".into(),
            steps: vec![Step {
                tool: "web_search".into(),
                input: serde_json::json!({"query": "q"}),
                output: "r".into(),
                error: false,
            }],
            input_tokens: 10,
            output_tokens: 2,
            cost: 0.001,
            seconds: 1.5,
        };
        write_export(&dir.path().join("out"), &export).unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &crate::chat::at_rest::read_to_string(dir.path().join("out/export.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                v["ending"].as_str(),
                v["exit_code"].as_i64(),
                v["steps"][0]["tool"].as_str()
            ),
            (Some("limit"), Some(3), Some("web_search"))
        );
    }
}
