//! Turning rig's message history into the turns and view the window shows, and a project's digest.

use super::*;

pub(super) fn result_text(content: &[ToolResultContent]) -> String {
    content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(t) => t.text.clone(),
            ToolResultContent::Json { value, .. } => value.to_string(),
            _ => "[non-text result]".to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Flattens rig's history into turns. Tool results, which rig carries as user messages, fold into
/// the assistant turn that made the call, so a turn reads: thought, tool, tool, answer.
pub fn turns(conversation: &Conversation) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    let time_of = |i: usize| {
        conversation
            .times
            .iter()
            .find(|(k, _)| *k == i)
            .map(|(_, t)| *t)
    };
    let rating_of = |i: usize| {
        conversation
            .ratings
            .iter()
            .find(|(k, _)| *k == i)
            .map_or(0, |(_, r)| *r)
    };
    for (index, message) in conversation.messages.iter().enumerate() {
        match message {
            Message::System { .. } => {}
            Message::User { content } => {
                let mut texts = Vec::new();
                for item in content {
                    match item {
                        UserContent::Text(t) => texts.push(Part::Text {
                            text: t.text.clone(),
                        }),
                        UserContent::Image(image) => {
                            if let rig_core::completion::message::DocumentSourceKind::Base64(data) =
                                &image.data
                            {
                                use rig_core::completion::message::MimeType;
                                texts.push(Part::Image {
                                    media_type: image
                                        .media_type
                                        .as_ref()
                                        .map_or("image/png", |m| m.to_mime_type())
                                        .to_string(),
                                    data: data.clone(),
                                });
                            }
                        }
                        UserContent::ToolResult(result) => {
                            let call = result.call.to_string();
                            let tool =
                                out.last_mut()
                                    .filter(|t| t.role == "assistant")
                                    .and_then(|turn| {
                                        turn.parts.iter_mut().find(|p| {
                                    matches!(p, Part::Tool { call_id, .. } if *call_id == call)
                                })
                                    });
                            if let Some(Part::Tool {
                                output, is_error, ..
                            }) = tool
                            {
                                *output = Some(result_text(&result.content));
                                *is_error = result.is_error;
                            }
                            if let Some(turn) = out.last_mut() {
                                turn.index = index;
                            }
                        }
                        _ => {}
                    }
                }
                if !texts.is_empty() {
                    out.push(Turn {
                        role: "user",
                        parts: texts,
                        index,
                        time: time_of(index),
                        rating: 0,
                    });
                }
            }
            Message::Assistant(assistant) => {
                let parts = assistant.content.iter().filter_map(|c| match c {
                    AssistantContent::Text(t) if !t.text.is_empty() => Some(Part::Text {
                        text: t.text.clone(),
                    }),
                    AssistantContent::Reasoning(r) if !r.text.is_empty() => Some(Part::Reasoning {
                        text: r.text.clone(),
                        ms: conversation
                            .thought
                            .iter()
                            .find(|(k, _)| *k == index)
                            .map(|(_, ms)| *ms),
                    }),
                    AssistantContent::ToolCall(call) => Some(Part::Tool {
                        call_id: call.id.to_string(),
                        name: call.function.name.to_string(),
                        arguments: Value::Object(call.function.arguments.clone()),
                        output: None,
                        is_error: false,
                    }),
                    _ => None,
                });
                let notices = conversation
                    .notices
                    .iter()
                    .filter(|(k, _)| *k == index)
                    .map(|(_, text)| Part::Notice { text: text.clone() });
                let parts: Vec<Part> = notices.chain(parts).collect();
                match out.last_mut() {
                    Some(turn) if turn.role == "assistant" => {
                        turn.parts.extend(parts);
                        turn.index = index;
                    }
                    _ => out.push(Turn {
                        role: "assistant",
                        parts,
                        index,
                        time: None,
                        rating: 0,
                    }),
                }
            }
        }
    }
    // A note made after the last answer (a skill learned from it) belongs to no message: it
    // closes the last answer. While a turn runs, the last message is the user's, and the live
    // turn shows its notes itself.
    if let Some(turn) = out.last_mut().filter(|t| t.role == "assistant") {
        turn.parts.extend(
            conversation
                .notices
                .iter()
                .filter(|(k, _)| *k >= conversation.messages.len())
                .map(|(_, text)| Part::Notice { text: text.clone() }),
        );
    }
    for turn in out.iter_mut().filter(|t| t.role == "assistant") {
        turn.time = time_of(turn.index);
        turn.rating = rating_of(turn.index);
    }
    out
}

/// What the project's other conversations were about: titles and how each ended, for a new
/// conversation that may build on them.
pub fn project_digest(config_dir: &Path, project_id: &str, except: &str) -> Option<String> {
    let mut lines = Vec::new();
    for summary in list_conversations(config_dir)
        .into_iter()
        .filter(|c| c.project_id.as_deref() == Some(project_id) && c.id != except)
        .take(8)
    {
        let Ok(c) = load_conversation(config_dir, &summary.id) else {
            continue;
        };
        let last = c.messages.iter().rev().find_map(|m| match m {
            Message::Assistant(a) => a.content.iter().find_map(|x| match x {
                AssistantContent::Text(t) if !t.text.trim().is_empty() => Some(t.text.clone()),
                _ => None,
            }),
            _ => None,
        });
        let excerpt: String = last
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(400)
            .collect();
        lines.push(format!(
            "- \"{}\" ({}): {}",
            summary.title,
            summary.updated_at.format("%Y-%m-%d"),
            excerpt
        ));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub fn view(conversation: &Conversation) -> ConversationView {
    ConversationView {
        id: conversation.id.clone(),
        usage: conversation.usage.clone(),
        compacted_before: conversation.summary.as_ref().map(|s| s.upto),
        title: conversation.title.clone(),
        project_id: conversation.project_id.clone(),
        workdir: conversation.workdir.clone(),
        agent_id: conversation.agent_id.clone(),
        model: conversation.model.clone(),
        pinned: conversation.pinned,
        archived: conversation.archived,
        params: conversation.params.clone(),
        bookmarks: conversation.bookmarks.iter().map(|(i, _)| *i).collect(),
        turns: turns(conversation),
        versions: conversation
            .versions
            .iter()
            .map(|v| {
                let mut v = v.clone();
                if v.kind.is_empty() {
                    v.from += 1;
                    v.kind = "answer".into();
                }
                v
            })
            .chain(conversation.branches.iter().cloned())
            .filter(|v| v.start() <= conversation.messages.len() && !v.others.is_empty())
            .map(|v| VersionsView {
                from: v.start(),
                kind: v.kind.clone(),
                current: v.current,
                total: v.others.len() + 1,
            })
            .collect(),
    }
}

/// What was said in a conversation, as plain text, cut at `limit` characters.
pub fn chat_transcript(conversation: &Conversation, limit: usize) -> String {
    let mut text = format!(
        "# {}
Date: {}
\n",
        conversation.title,
        conversation.created_at.format("%Y-%m-%d")
    );
    for turn in turns(conversation) {
        let said: Vec<String> = turn
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.clone()),
                Part::Tool { name, .. } => Some(format!("[used {name}]")),
                _ => None,
            })
            .collect();
        if said.is_empty() {
            continue;
        }
        text.push_str(if turn.role == "user" {
            "User: "
        } else {
            "Assistant: "
        });
        text.push_str(&said.join("\n"));
        text.push_str("\n\n");
        if text.chars().count() > limit {
            let cut: String = crate::text::clip(&text, limit);
            return format!("{cut}\n\n[The rest of the conversation is cut off.]");
        }
    }
    text
}
