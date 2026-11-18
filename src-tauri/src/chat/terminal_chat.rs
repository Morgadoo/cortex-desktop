//! A chat in the terminal (Vibe Code's CLI): `lumo-desktop chat` talks with the app's models in
//! the terminal, streaming each reply as it is written, with the read-only tools (web search,
//! libraries, knowledge, earlier chats, calendars, contacts). Every conversation is saved to the
//! app's history, so it can be opened in the window later (`/open` prints its link).
//!
//! ```text
//! lumo-desktop chat [--model vendor/model] [--resume <conversation id>]
//! ```
//!
//! Slash commands: `/help`, `/model [ref]`, `/models`, `/think fast|low|think|deep`,
//! `/tools on|off`, `/new`, `/open`, `/save <file.md>`, `/exit`. A line ending in `\` continues on
//! the next one.

use std::io::{BufRead as _, Write as _};
use std::path::Path;

use futures::StreamExt;
use rig_core::completion::message::{ToolResultContent, UserContent};
use rig_core::completion::{CompletionRequest, Message};
use rig_core::streaming::{Item, StreamEvent};

use super::builtins;
use super::engine::Speed;
use super::store::{self, Conversation};
use super::tools::ChatTool;

const MAX_STEPS: usize = 8;

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

const HELP: &str = "Commands:
  /model [vendor/model]   show or change the model
  /models                 list the models set up in the app
  /think fast|low|think|deep   how much the model reasons
  /tools on|off           let the model search the web, your libraries and knowledge
  /new                    start a new conversation
  /open                   the link that opens this conversation in the app
  /save <file.md>         save the conversation as Markdown
  /exit                   leave (Ctrl+D works too)
A line ending in \\ continues on the next line.";

/// A slash command, read from one line.
#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Model(Option<String>),
    Models,
    Think(Speed),
    Tools(bool),
    New,
    Open,
    Save(String),
    Exit,
    Unknown(String),
}

pub fn command(line: &str) -> Option<Command> {
    let line = line.trim();
    let rest = line.strip_prefix('/')?;
    let (name, arg) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(n, a)| (n, a.trim()));
    Some(match name {
        "help" | "?" => Command::Help,
        "model" => Command::Model(Some(arg.to_string()).filter(|a| !a.is_empty())),
        "models" => Command::Models,
        "think" => match arg {
            "fast" | "off" => Command::Think(Speed::Fast),
            "low" => Command::Think(Speed::Low),
            "" | "think" | "high" => Command::Think(Speed::Think),
            "deep" | "xhigh" | "harder" => Command::Think(Speed::Deep),
            other => Command::Unknown(format!("/think {other}: use fast, low, think or deep")),
        },
        "tools" => match arg {
            "on" | "" => Command::Tools(true),
            "off" => Command::Tools(false),
            other => Command::Unknown(format!("/tools {other}: use on or off")),
        },
        "new" | "clear" => Command::New,
        "open" => Command::Open,
        "save" if !arg.is_empty() => Command::Save(arg.to_string()),
        "save" => Command::Unknown("/save needs a file name, like /save notes.md".into()),
        "exit" | "quit" | "q" => Command::Exit,
        other => Command::Unknown(format!("unknown command /{other}; /help lists them")),
    })
}

/// The conversation as Markdown, for `/save`.
pub fn markdown(c: &Conversation) -> String {
    let mut out = format!("# {}\n\n", if c.title.is_empty() { "Conversation" } else { &c.title });
    for m in &c.messages {
        // Tool results travel as user messages; only what was said is kept.
        let is_tool = matches!(m, Message::User { content } if content.iter().all(|p| !matches!(p, UserContent::Text(_))));
        let text = super::context::message_text(m);
        if text.trim().is_empty() || is_tool {
            continue;
        }
        let who = if matches!(m, Message::User { .. }) { "You" } else { "Lumo" };
        out.push_str(&format!("**{who}:** {}\n\n", text.trim()));
    }
    out
}

fn read_line(prompt: &str) -> Option<String> {
    let stdin = std::io::stdin();
    let mut text = String::new();
    let mut first = true;
    loop {
        print!("{}", if first { prompt } else { "… " });
        let _ = std::io::stdout().flush();
        first = false;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).ok()? == 0 {
            return if text.is_empty() { None } else { Some(text) };
        }
        let line = line.trim_end_matches(['\n', '\r']);
        match line.strip_suffix('\\') {
            Some(part) => {
                text.push_str(part);
                text.push('\n');
            }
            None => {
                text.push_str(line);
                return Some(text);
            }
        }
    }
}

/// Runs `chat` when asked; `None` to start the app.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("chat") {
        return None;
    }
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    Some(tauri::async_runtime::block_on(run(value("--model"), value("--resume"))))
}

async fn run(model_arg: Option<String>, resume: Option<String>) -> i32 {
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        eprintln!("lumo: the app's config folder could not be found");
        return 1;
    };
    let mut conversation = match resume {
        Some(id) => match store::load_conversation(&config_dir, &id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("lumo: {e}");
                return 1;
            }
        },
        None => Conversation::new(uuid::Uuid::new_v4().to_string(), None, None),
    };
    let mut reference = match model_arg.or_else(|| conversation.model.clone()).or_else(|| super::providers::load(&config_dir).default_model) {
        Some(r) => r,
        None => {
            eprintln!("lumo: no model: pass --model vendor/model, or choose one in the app first");
            return 1;
        }
    };
    let mut model = match super::engine::connect_model(&config_dir, &reference).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("lumo: {e}");
            return 1;
        }
    };
    let mut speed = Speed::Fast;
    let mut use_tools = true;
    println!("Lumo · {reference} · /help for commands, /exit to leave");
    if !conversation.messages.is_empty() {
        println!("Resuming “{}” ({} messages).", conversation.title, conversation.messages.len());
    }
    loop {
        let Some(line) = read_line("› ") else {
            println!();
            return 0;
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(cmd) = command(&line) {
            match cmd {
                Command::Help => println!("{HELP}"),
                Command::Exit => return 0,
                Command::Model(None) => println!("{reference}"),
                Command::Model(Some(r)) => match super::engine::connect_model(&config_dir, &r).await {
                    Ok(m) => {
                        model = m;
                        reference = r;
                        println!("Now using {reference}.");
                    }
                    Err(e) => println!("{e}"),
                },
                Command::Models => {
                    for m in super::providers::models(&super::providers::load(&config_dir)).await {
                        println!("  {}{}", m.reference, if m.reference == reference { "  (in use)" } else { "" });
                    }
                }
                Command::Think(s) => {
                    speed = s;
                    println!("Thinking: {}.", match s { Speed::Fast => "fast, no extended reasoning", Speed::Low => "a little", Speed::Think => "for complex tasks", Speed::Deep => "harder" });
                }
                Command::Tools(on) => {
                    use_tools = on;
                    println!("Tools {}.", if on { "on" } else { "off" });
                }
                Command::New => {
                    conversation = Conversation::new(uuid::Uuid::new_v4().to_string(), None, None);
                    println!("New conversation.");
                }
                Command::Open => println!("lumo://chat/{}", conversation.id),
                Command::Save(path) => match crate::chat::at_rest::write(&path, markdown(&conversation)) {
                    Ok(()) => println!("Saved to {path}."),
                    Err(e) => println!("Could not save: {e}"),
                },
                Command::Unknown(msg) => println!("{msg}"),
            }
            continue;
        }
        if conversation.title.is_empty() {
            conversation.title = line.lines().next().unwrap_or_default().chars().take(60).collect();
        }
        conversation.model = Some(reference.clone());
        conversation.messages.push(Message::user(line.clone()));
        conversation.times.push((conversation.messages.len() - 1, chrono::Utc::now()));
        if let Err(e) = turn(&config_dir, &model, &mut conversation, speed, use_tools).await {
            println!("\n! {e}");
        }
        conversation.updated_at = chrono::Utc::now();
        if let Err(e) = store::save_conversation(&config_dir, &conversation) {
            eprintln!("lumo: could not save the conversation: {e}");
        }
    }
}

/// One reply, streamed to the terminal, with tool steps between.
async fn turn(config_dir: &Path, model: &rig_core::DynModel<rig_core::operation::Completion>, conversation: &mut Conversation, speed: Speed, use_tools: bool) -> Result<(), String> {
    let tools: Vec<ChatTool> = if use_tools { builtins::definitions(config_dir).into_iter().filter(|t| READ_ONLY.contains(&t.name.as_str())).collect() } else { Vec::new() };
    let system = super::headless::preamble(config_dir).replace("answering one request from the command line. Nobody can answer questions back, so make reasonable assumptions and state them.", "chatting with the user in a terminal. Keep answers readable as plain text.");
    for step in 0..=MAX_STEPS {
        let last = step == MAX_STEPS;
        let request_messages = super::context::request_messages(&conversation.messages, conversation.summary.as_ref());
        let Some((prompt, history)) = request_messages.split_last() else { return Ok(()) };
        let mut request = CompletionRequest::new(prompt.clone()).messages(history.iter().cloned()).preamble(system.clone());
        if !last && !tools.is_empty() {
            request = request.tools(tools.iter().filter_map(ChatTool::definition));
        }
        if let Some(effort) = speed.effort() {
            request = request.reasoning(effort);
        }
        // Dropped connections and overloaded servers are retried, as in the app, while nothing
        // has been printed yet.
        let mut attempt = 0u32;
        let (response, printed) = loop {
            attempt += 1;
            let mut stream = model.stream(request.clone()).map_err(|e| e.to_string())?;
            let mut printed = false;
            let mut thinking = false;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(Item::Event(StreamEvent::Reasoning { .. })) if !thinking && !printed => {
                        thinking = true;
                        print!("\x1b[2m(thinking…)\x1b[0m ");
                        let _ = std::io::stdout().flush();
                    }
                    Ok(Item::Event(StreamEvent::Text { text, .. })) if !text.is_empty() => {
                        if thinking {
                            print!("\r\x1b[2K");
                            thinking = false;
                        }
                        printed = true;
                        print!("{text}");
                        let _ = std::io::stdout().flush();
                    }
                    _ => {}
                }
            }
            match stream.finish().await {
                Ok(r) => break (r, printed),
                Err(e) if e.is_retryable() && !printed && attempt < 4 => {
                    if thinking {
                        print!("\r\x1b[2K");
                    }
                    println!("\x1b[2m(retrying: {})\x1b[0m", super::context::brief_error(&e.to_string()));
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                }
                Err(e) => return Err(super::context::short_error(&e.to_string())),
            }
        };
        if printed {
            println!();
        }
        conversation.usage.add(&response.usage);
        let calls: Vec<_> = response.tool_calls().cloned().collect();
        if let Some(message) = response.message() {
            conversation.messages.push(message);
        }
        if calls.is_empty() {
            conversation.times.push((conversation.messages.len() - 1, chrono::Utc::now()));
            return Ok(());
        }
        let mut results = Vec::new();
        for call in calls {
            let name = call.function.name.as_str().to_string();
            let outcome = if READ_ONLY.contains(&name.as_str()) {
                builtins::run(config_dir, &name, &serde_json::Value::Object(call.function.arguments.clone())).await
            } else {
                super::tools::Outcome { text: format!("{name} is not available in the terminal."), is_error: true }
            };
            println!("\x1b[2m  · {}{}\x1b[0m", name.replace('_', " "), if outcome.is_error { " (failed)" } else { "" });
            let mut result = call.result(vec![ToolResultContent::text(outcome.text)]);
            result.is_error = outcome.is_error;
            results.push(result);
        }
        conversation.messages.push(Message::tool_results(results));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_commands_are_read() {
        assert_eq!(command("hello"), None);
        assert_eq!(command("/model"), Some(Command::Model(None)));
        assert_eq!(command("/model xai/grok-4"), Some(Command::Model(Some("xai/grok-4".into()))));
        assert_eq!(command("/think deep"), Some(Command::Think(Speed::Deep)));
        assert_eq!(command("/think off"), Some(Command::Think(Speed::Fast)));
        assert_eq!(command("/tools off"), Some(Command::Tools(false)));
        assert_eq!(command("/save notes.md"), Some(Command::Save("notes.md".into())));
        assert!(matches!(command("/save"), Some(Command::Unknown(_))));
        assert!(matches!(command("/nope"), Some(Command::Unknown(_))));
        assert_eq!(command(" /exit "), Some(Command::Exit));
    }

    #[test]
    fn conversations_save_as_markdown() {
        let mut c = Conversation::new("c".into(), None, None);
        c.title = "Trip".into();
        c.messages.push(Message::user("Where to?"));
        c.messages.push(Message::assistant("Porto."));
        assert_eq!(markdown(&c), "# Trip\n\n**You:** Where to?\n\n**Lumo:** Porto.\n\n");
    }
}
