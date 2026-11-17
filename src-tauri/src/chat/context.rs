//! Context engineering: what of a conversation each request actually carries.
//!
//! The stored history is complete; a request gets a budgeted view of it:
//! - **Compaction.** When a conversation nears the model's context window, the turns before the
//!   last few are summarized once into a structured note that rides in the system prompt, and
//!   requests start after it. The note is refreshed when it falls behind again.
//! - **Tool-result clearing.** Only the most recent tool results go in full; older ones are cut
//!   to a short head with a marker, since the model already acted on them and can run the tool
//!   again. Even recent ones are capped, so one huge page cannot crowd out the rest.
//! - **Measured, not guessed.** The provider's reported context tokens from the last reply decide
//!   when to compact; a character estimate stands in only before the first reply.

use chrono::{DateTime, Utc};
use rig_core::catalog::Catalog;
use rig_core::completion::message::{Message, ToolResultContent, UserContent};
use serde::{Deserialize, Serialize};

/// The most recent tool results sent in full.
pub const KEEP_FULL_RESULTS: usize = 8;
/// Characters kept of an older tool result.
pub const CLEARED_HEAD: usize = 600;
/// Characters a recent tool result may take at most.
pub const MAX_RESULT_CHARS: usize = 24_000;
/// Compact once the context passes this share of the window.
pub const COMPACT_AT: f64 = 0.6;
/// User turns kept verbatim after the compaction point.
pub const KEEP_RECENT_TURNS: usize = 2;
/// The window assumed for a model the catalog does not size.
pub const DEFAULT_WINDOW: u64 = 128_000;

/// The structured note that stands in for the turns before `upto`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub text: String,
    /// Index of the first message requests still carry verbatim.
    pub upto: usize,
    pub created_at: DateTime<Utc>,
}

/// Tokens spent in a conversation, as providers reported them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// In US dollars, when the catalog prices the model.
    pub cost: f64,
    /// What the latest request carried, which the next one sends again.
    pub context_tokens: u64,
}

impl UsageTotals {
    pub fn add(&mut self, usage: &rig_core::completion::Usage) {
        self.input_tokens += usage.input_tokens.unwrap_or(0);
        self.output_tokens += usage.output_tokens.unwrap_or(0);
        self.cost += usage.cost.map_or(0.0, |c| c.total);
        if let Some(context) = usage.context_tokens() {
            self.context_tokens = context;
        }
    }
}

pub fn context_window(reference: &str) -> u64 {
    Catalog::builtin()
        .resolve(reference)
        .ok()
        .and_then(|r| r.spec.max_input_tokens.or(r.spec.context_window))
        .map_or(DEFAULT_WINDOW, u64::from)
}

/// A rough token count: four characters a token, over the JSON the provider would receive.
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|m| serde_json::to_string(m).map_or(0, |s| s.len() as u64))
        .sum::<u64>()
        / 4
}

fn is_prompt(message: &Message) -> bool {
    matches!(message, Message::User { content } if content.iter().any(|c| matches!(c, UserContent::Text(_))))
}

/// Where a compaction would cut: the start of the `KEEP_RECENT_TURNS`-th last prompt, if that is
/// past what the current summary covers. Cutting at a prompt keeps every tool call with its result.
pub fn compaction_point(messages: &[Message], covered: usize) -> Option<usize> {
    let prompts: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| is_prompt(m)).map(|(i, _)| i).collect();
    let point = *prompts.iter().rev().nth(KEEP_RECENT_TURNS - 1)?;
    (point > covered).then_some(point)
}

pub fn should_compact(context_tokens: u64, window: u64) -> bool {
    context_tokens as f64 > window as f64 * COMPACT_AT
}

fn clip(text: &str, max: usize, note: &str) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\n[… {note}]")
}

/// The messages a request carries: from the summary on, with older tool results cleared.
pub fn request_messages(messages: &[Message], summary: Option<&Summary>) -> Vec<Message> {
    let start = summary.map_or(0, |s| s.upto.min(messages.len()));
    let mut out: Vec<Message> = messages[start..].to_vec();
    let mut seen = 0usize;
    for message in out.iter_mut().rev() {
        let Message::User { content } = message else { continue };
        for item in content.iter_mut().rev() {
            let UserContent::ToolResult(result) = item else { continue };
            seen += 1;
            let old = seen > KEEP_FULL_RESULTS;
            for part in result.content.iter_mut() {
                if let ToolResultContent::Text(text) = part {
                    let total = text.text.chars().count();
                    text.text = if old {
                        clip(&text.text, CLEARED_HEAD, &format!("older result cleared, {total} characters in all; run the tool again if you need it"))
                    } else {
                        clip(&text.text, MAX_RESULT_CHARS, &format!("cut at {MAX_RESULT_CHARS} of {total} characters"))
                    };
                }
            }
        }
    }
    out
}

/// The first line of an error, short enough for a notice.
pub fn short_error(text: &str) -> String {
    // A provider's JSON error reads better as just its message.
    if let Some(m) = regex::Regex::new(r#""message"\s*:\s*"([^"]{1,200})""#).ok().and_then(|re| re.captures(text)) {
        return m[1].to_string();
    }
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 120 { format!("{}…", line.chars().take(120).collect::<String>()) } else { line.to_string() }
}

/// One slice of the context window.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Slice {
    pub label: String,
    pub tokens: u64,
}

/// What fills the model's window, for the context view.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub window: u64,
    /// What the latest request carried, as the provider measured it; 0 before the first reply.
    pub measured: u64,
    /// Estimates (four characters a token), largest first.
    pub slices: Vec<Slice>,
    /// Messages replaced by the summary, and tool results shortened to save room.
    pub summarized_messages: usize,
    pub cleared_results: usize,
}

/// Splits what the next request carries into what it is made of.
pub fn breakdown(messages: &[Message], summary: Option<&Summary>, measured: u64, window: u64) -> Breakdown {
    use rig_core::completion::message::AssistantContent;
    let size = |text: &str| text.len() as u64 / 4;
    let (mut you, mut files, mut answers, mut thinking, mut calls, mut results) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let request = request_messages(messages, summary);
    for message in &request {
        match message {
            Message::User { content } => {
                for item in content.iter() {
                    match item {
                        UserContent::Text(t) => you += size(&t.text),
                        UserContent::ToolResult(r) => results += size(&serde_json::to_string(r).unwrap_or_default()),
                        // An image costs by its size in pixels, not its bytes; a typical one is about this.
                        UserContent::Image(_) => files += 1_000,
                        other => files += size(&serde_json::to_string(other).unwrap_or_default()),
                    }
                }
            }
            Message::Assistant(a) => {
                for item in a.content.iter() {
                    match item {
                        AssistantContent::Text(t) => answers += size(&t.text),
                        AssistantContent::ToolCall(c) => calls += size(&serde_json::to_string(c).unwrap_or_default()),
                        other => thinking += size(&serde_json::to_string(other).unwrap_or_default()),
                    }
                }
            }
            _ => {}
        }
    }
    let note = summary.map_or(0, |s| s.text.len() as u64 / 4);
    let counted = you + files + answers + thinking + calls + results + note;
    let cleared_results = {
        let total = request.iter().filter_map(|m| match m { Message::User { content } => Some(content.iter().filter(|c| matches!(c, UserContent::ToolResult(_))).count()), _ => None }).sum::<usize>();
        total.saturating_sub(KEEP_FULL_RESULTS)
    };
    let mut slices: Vec<Slice> = [
        ("Instructions, knowledge, skills and tool definitions", measured.saturating_sub(counted)),
        ("Your messages", you),
        ("Answers", answers),
        ("Tool results", results),
        ("Tool calls", calls),
        ("Thinking", thinking),
        ("Images and files", files),
        ("Summary of earlier messages", note),
    ]
    .into_iter()
    .filter(|(_, tokens)| *tokens > 0)
    .map(|(label, tokens)| Slice { label: label.into(), tokens })
    .collect();
    slices.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    Breakdown { window, measured, slices, summarized_messages: summary.map_or(0, |s| s.upto.min(messages.len())), cleared_results }
}

/// The instruction that turns old turns into a note the conversation can continue from.
pub fn summary_prompt(previous: Option<&str>, transcript: &str) -> String {
    let previous = previous.map_or(String::new(), |p| format!("The earlier summary, to fold in:\n{p}\n\n"));
    format!(
        "Summarize the conversation below so it can continue without it. Keep what later turns \
         need and drop the rest. Use these headings, skipping empty ones:\n\
         ## Goal\n## Facts and figures (with source URLs)\n## Decisions and preferences\n\
         ## Work done (files, commands, results)\n## Open questions and next steps\n\
         Write at most 500 words, in the conversation's language.\n\n{previous}Conversation:\n{transcript}"
    )
}

/// A plain-text rendering of messages for the summarizer, with long tool results cut.
pub fn transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        match message {
            Message::System { .. } => {}
            Message::User { content } => {
                for item in content {
                    match item {
                        UserContent::Text(t) => out.push_str(&format!("User: {}\n\n", t.text)),
                        UserContent::ToolResult(r) => {
                            let text: String = r
                                .content
                                .iter()
                                .filter_map(|c| match c {
                                    ToolResultContent::Text(t) => Some(t.text.as_str()),
                                    _ => None,
                                })
                                .collect();
                            out.push_str(&format!("Tool result ({}): {}\n\n", r.name, clip(&text, 1500, "cut")));
                        }
                        _ => {}
                    }
                }
            }
            Message::Assistant(a) => {
                for item in &a.content {
                    match item {
                        rig_core::completion::message::AssistantContent::Text(t) => {
                            out.push_str(&format!("Assistant: {}\n\n", t.text))
                        }
                        rig_core::completion::message::AssistantContent::ToolCall(c) => out.push_str(&format!(
                            "Assistant called {} with {}\n\n",
                            c.function.name,
                            serde_json::Value::Object(c.function.arguments.clone())
                        )),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::completion::message::{AssistantContent, AssistantMessage, CallId, ToolName};

    fn result(id: &str, text: &str) -> Message {
        Message::tool_result(CallId::from_wire(id), ToolName::new("web_search").unwrap(), text)
    }

    fn call(id: &str) -> Message {
        Message::Assistant(AssistantMessage::new(vec![AssistantContent::tool_call(
            id,
            ToolName::new("web_search").unwrap(),
            serde_json::json!({}),
        )]))
    }

    fn result_text(m: &Message) -> &str {
        let Message::User { content } = m else { panic!() };
        let UserContent::ToolResult(r) = &content[0] else { panic!() };
        let ToolResultContent::Text(t) = &r.content[0] else { panic!() };
        &t.text
    }

    #[test]
    fn old_tool_results_are_cleared_and_recent_ones_capped() {
        let mut messages = vec![Message::user("go")];
        for i in 0..KEEP_FULL_RESULTS + 2 {
            messages.push(call(&format!("c{i}")));
            messages.push(result(&format!("c{i}"), &"x".repeat(2000)));
        }
        messages.push(call("big"));
        messages.push(result("big", &"y".repeat(MAX_RESULT_CHARS + 10)));
        let out = request_messages(&messages, None);
        assert_eq!(out.len(), messages.len());
        assert!(result_text(&out[2]).contains("older result cleared"));
        assert!(result_text(&out[2]).len() < 800);
        let recent = result_text(&out[out.len() - 3]);
        assert_eq!(recent.len(), 2000);
        assert!(result_text(&out[out.len() - 1]).contains("cut at"));
    }

    #[test]
    fn a_summary_starts_requests_after_what_it_covers() {
        let messages = vec![Message::user("a"), Message::assistant("b"), Message::user("c"), Message::assistant("d")];
        let summary = Summary { text: "s".into(), upto: 2, created_at: Utc::now() };
        let out = request_messages(&messages, Some(&summary));
        assert_eq!(out, messages[2..].to_vec());
    }

    #[test]
    fn compaction_cuts_at_a_prompt_and_keeps_recent_turns() {
        let messages = vec![
            Message::user("1"), Message::assistant("a"),
            Message::user("2"), call("x"), result("x", "r"), Message::assistant("b"),
            Message::user("3"), Message::assistant("c"),
        ];
        assert_eq!(compaction_point(&messages, 0), Some(2));
        assert_eq!(compaction_point(&messages, 2), None);
        assert_eq!(compaction_point(&messages[..2], 0), None);
        assert!(should_compact(70_000, 100_000));
        assert!(!should_compact(50_000, 100_000));
    }

    #[test]
    fn transcripts_and_prompts_carry_the_essentials() {
        let messages = vec![Message::user("Plan Lisbon"), call("x"), result("x", "tram 28"), Message::assistant("Day 1")];
        let text = transcript(&messages);
        assert!(text.contains("User: Plan Lisbon"));
        assert!(text.contains("Tool result (web_search): tram 28"));
        assert!(text.contains("Assistant: Day 1"));
        let prompt = summary_prompt(Some("old note"), &text);
        assert!(prompt.contains("old note") && prompt.contains("## Goal"));
        assert!(context_window("openai/gpt-5.5") >= 100_000);
        assert_eq!(context_window("nobody/nothing"), DEFAULT_WINDOW);
        assert!(estimate_tokens(&messages) > 0);
        assert_eq!(short_error(r#"ProviderResponseError: status 400: {"error":{"code":"model_not_found","message":"unknown provider for model x"}}"#), "unknown provider for model x");
        // The context view: what is counted is named, and the rest is the instructions and tools.
        let view = breakdown(&messages, None, 50_000, 128_000);
        assert_eq!(view.slices[0].label, "Instructions, knowledge, skills and tool definitions");
        assert!(view.slices.iter().any(|s| s.label == "Your messages"));
        assert_eq!((view.window, view.measured, view.summarized_messages), (128_000, 50_000, 0));
    }
}

/// The text of a message: the user's typed words, or the assistant's reply.
pub fn message_text(message: &Message) -> String {
    use rig_core::completion::message::{AssistantContent, UserContent};
    match message {
        Message::User { content } => content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant(a) => a
            .content
            .iter()
            .filter_map(|c| match c {
                AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
