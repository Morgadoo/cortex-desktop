//! A chat in the terminal (Vibe Code's CLI): `cortex-desktop chat` talks with the app's models in
//! the terminal, streaming each reply as it is written, with the read-only tools (web search,
//! libraries, knowledge, earlier chats, calendars, contacts). Every conversation is saved to the
//! app's history, so it can be opened in the window later (`/open` prints its link).
//!
//! ```text
//! cortex-desktop chat [--model vendor/model] [--resume <conversation id>]
//! ```
//!
//! Slash commands: `/help`, `/model [ref]`, `/models`, `/think fast|low|think|deep`,
//! `/tools on|off`, `/new`, `/open`, `/save <file.md>`, `/exit`. A line ending in `\` continues on
//! the next one.

use std::io::{BufRead as _, Write as _};
use std::path::Path;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rig_core::completion::message::UserContent;
use rig_core::completion::Message;

use crate::chat::builtins;
use crate::chat::engine::{ChatEvent, Speed, TurnRequest};
use crate::chat::host::Host;
use crate::chat::store::{self, Conversation};
use crate::chat::tools::{ApprovalMode, ChatTool};
use crate::tools::registry::ApprovalDecision;

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

pub(in crate::chat::cli) fn command(line: &str) -> Option<Command> {
    let line = line.trim();
    let rest = line.strip_prefix('/')?;
    let (name, arg) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(n, a)| (n, a.trim()));
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
pub(in crate::chat::cli) fn markdown(c: &Conversation) -> String {
    let mut out = format!(
        "# {}\n\n",
        if c.title.is_empty() {
            "Conversation"
        } else {
            &c.title
        }
    );
    for m in &c.messages {
        // Tool results travel as user messages; only what was said is kept.
        let is_tool = matches!(m, Message::User { content } if content.iter().all(|p| !matches!(p, UserContent::Text(_))));
        let text = crate::chat::context::message_text(m);
        if text.trim().is_empty() || is_tool {
            continue;
        }
        let who = if matches!(m, Message::User { .. }) {
            "You"
        } else {
            "Cortex"
        };
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
pub(crate) fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("chat") {
        return None;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    Some(tauri::async_runtime::block_on(run(
        value("--model"),
        value("--resume"),
    )))
}

async fn run(model_arg: Option<String>, resume: Option<String>) -> i32 {
    let Some(config_dir) = crate::reporting::early_config_dir() else {
        eprintln!("cortex: the app's config folder could not be found");
        return 1;
    };
    let mut conversation = match resume {
        Some(id) => match store::load_conversation(&config_dir, &id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cortex: {e}");
                return 1;
            }
        },
        None => Conversation::new(uuid::Uuid::new_v4().to_string(), None, None),
    };
    let mut reference = match model_arg
        .or_else(|| conversation.model.clone())
        .or_else(|| crate::chat::providers::load(&config_dir).default_model)
    {
        Some(r) => r,
        None => {
            eprintln!(
                "cortex: no model: pass --model vendor/model, or choose one in the app first"
            );
            return 1;
        }
    };
    if let Err(e) = crate::chat::engine::connect_model(&config_dir, &reference).await {
        eprintln!("cortex: {e}");
        return 1;
    }
    let mut speed = Speed::Fast;
    let mut use_tools = true;
    println!("Cortex · {reference} · /help for commands, /exit to leave");
    if !conversation.messages.is_empty() {
        println!(
            "Resuming “{}” ({} messages).",
            conversation.title,
            conversation.messages.len()
        );
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
                Command::Model(Some(r)) => {
                    match crate::chat::engine::connect_model(&config_dir, &r).await {
                        Ok(_) => {
                            reference = r;
                            println!("Now using {reference}.");
                        }
                        Err(e) => println!("{e}"),
                    }
                }
                Command::Models => {
                    for m in
                        crate::chat::providers::models(&crate::chat::providers::load(&config_dir))
                            .await
                    {
                        println!(
                            "  {}{}",
                            m.reference,
                            if m.reference == reference {
                                "  (in use)"
                            } else {
                                ""
                            }
                        );
                    }
                }
                Command::Think(s) => {
                    speed = s;
                    println!(
                        "Thinking: {}.",
                        match s {
                            Speed::Fast => "fast, no extended reasoning",
                            Speed::Low => "a little",
                            Speed::Think => "for complex tasks",
                            Speed::Deep => "harder",
                        }
                    );
                }
                Command::Tools(on) => {
                    use_tools = on;
                    println!("Tools {}.", if on { "on" } else { "off" });
                }
                Command::New => {
                    conversation = Conversation::new(uuid::Uuid::new_v4().to_string(), None, None);
                    println!("New conversation.");
                }
                Command::Open => println!("cortex://chat/{}", conversation.id),
                Command::Save(path) => {
                    match crate::chat::at_rest::write(&path, markdown(&conversation)) {
                        Ok(()) => println!("Saved to {path}."),
                        Err(e) => println!("Could not save: {e}"),
                    }
                }
                Command::Unknown(msg) => println!("{msg}"),
            }
            continue;
        }
        if conversation.title.is_empty() {
            conversation.title = line
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(60)
                .collect();
        }
        conversation.model = Some(reference.clone());
        if let Err(e) = store::save_conversation(&config_dir, &conversation) {
            eprintln!("cortex: could not save the conversation: {e}");
            continue;
        }
        if let Err(e) = turn(
            &config_dir,
            &reference,
            &conversation.id,
            &line,
            speed,
            use_tools,
        )
        .await
        {
            println!("\n! {e}");
        }
        // The engine saved the turn; this is what it wrote.
        if let Ok(latest) = store::load_conversation(&config_dir, &conversation.id) {
            conversation = latest;
        }
    }
}

/// Prints a turn as it runs: the reply as it streams, a note while the model thinks, and a line
/// per tool step.
struct TerminalHost {
    config_dir: std::path::PathBuf,
    state: crate::chat::ChatState,
    registry: crate::tools::registry::ToolRegistry,
    /// Call id → tool name, for the line printed when the call ends.
    calls: Mutex<HashMap<String, String>>,
    /// (thinking shown, reply text printed)
    printing: Mutex<(bool, bool)>,
}

#[async_trait::async_trait]
impl Host for TerminalHost {
    fn config_dir(&self) -> Result<std::path::PathBuf, crate::chat::error::ChatError> {
        Ok(self.config_dir.clone())
    }

    fn emit(&self, event: ChatEvent) {
        let mut printing = self.printing.lock().unwrap_or_else(|p| p.into_inner());
        let (thinking, printed) = &mut *printing;
        match event {
            ChatEvent::Reasoning { .. } if !*thinking && !*printed => {
                *thinking = true;
                print!("\x1b[2m(thinking…)\x1b[0m ");
            }
            ChatEvent::Text { text, .. } if !text.is_empty() => {
                if *thinking {
                    print!("\r\x1b[2K");
                    *thinking = false;
                }
                *printed = true;
                print!("{text}");
            }
            ChatEvent::ToolStart { call_id, name, .. } => {
                if *printed {
                    println!();
                    *printed = false;
                }
                self.calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(call_id, name);
            }
            ChatEvent::ToolEnd {
                call_id, is_error, ..
            } => {
                let name = self
                    .calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&call_id)
                    .unwrap_or_default();
                println!(
                    "\x1b[2m  · {}{}\x1b[0m",
                    name.replace('_', " "),
                    if is_error { " (failed)" } else { "" }
                );
            }
            ChatEvent::Notice { text, .. } => println!("\x1b[2m({text})\x1b[0m"),
            ChatEvent::Done { .. } | ChatEvent::Error { .. } => {}
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }

    fn chat_state(&self) -> &crate::chat::ChatState {
        &self.state
    }

    fn registry(&self) -> &crate::tools::registry::ToolRegistry {
        &self.registry
    }

    async fn connect_model(
        &self,
        reference: &str,
    ) -> Result<rig_core::DynModel<rig_core::operation::Completion>, crate::chat::error::ChatError>
    {
        crate::chat::engine::connect_model(&self.config_dir, reference).await
    }

    async fn approve(
        &self,
        _tool: &ChatTool,
        _input: &serde_json::Value,
        _reason: Option<&str>,
    ) -> ApprovalDecision {
        // The terminal offers only tools that read, which never ask.
        ApprovalDecision::Deny
    }

    fn notify(&self, title: &str, _body: &str) {
        println!("\x1b[2m({title})\x1b[0m");
    }
}

/// One reply, streamed to the terminal, with tool steps between, through the app's agent loop.
async fn turn(
    config_dir: &Path,
    reference: &str,
    conversation_id: &str,
    line: &str,
    speed: Speed,
    use_tools: bool,
) -> Result<(), String> {
    let host = Arc::new(TerminalHost {
        config_dir: config_dir.to_path_buf(),
        state: crate::chat::ChatState::default(),
        registry: crate::tools::registry::ToolRegistry::new(),
        calls: Mutex::new(HashMap::new()),
        printing: Mutex::new((false, false)),
    });
    let request = TurnRequest {
        conversation_id: conversation_id.to_string(),
        text: line.to_string(),
        model: reference.to_string(),
        speed,
        approval: ApprovalMode::Ask,
        connectors: Some(Vec::new()),
        max_steps: Some(MAX_STEPS + 1),
        only_tools: Some(if use_tools {
            READ_ONLY.iter().map(|t| t.to_string()).collect()
        } else {
            Vec::new()
        }),
        ..TurnRequest::default()
    };
    let cancel = host
        .state
        .begin(conversation_id)
        .ok_or("this conversation is busy")?;
    let dyn_host: Arc<dyn Host> = host.clone();
    let outcome = crate::chat::engine::turn(&dyn_host, &request, cancel).await;
    host.state.finish(conversation_id);
    if host.printing.lock().unwrap_or_else(|p| p.into_inner()).1 {
        println!();
    }
    outcome
        .map(|_| ())
        .map_err(|e| crate::chat::context::short_error(&e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_commands_are_read() {
        assert_eq!(command("hello"), None);
        assert_eq!(command("/model"), Some(Command::Model(None)));
        assert_eq!(
            command("/model xai/grok-4"),
            Some(Command::Model(Some("xai/grok-4".into())))
        );
        assert_eq!(command("/think deep"), Some(Command::Think(Speed::Deep)));
        assert_eq!(command("/think off"), Some(Command::Think(Speed::Fast)));
        assert_eq!(command("/tools off"), Some(Command::Tools(false)));
        assert_eq!(
            command("/save notes.md"),
            Some(Command::Save("notes.md".into()))
        );
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
        assert_eq!(
            markdown(&c),
            "# Trip\n\n**You:** Where to?\n\n**Cortex:** Porto.\n\n"
        );
    }
}
