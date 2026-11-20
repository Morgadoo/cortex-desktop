//! Titles for new chats, from the model or from the first message.

use super::*;

/// A message without the composer's directives ("Use a canvas…", "Use the … skill.", quoted
/// selections), which say how to answer rather than what the conversation is about.
pub(in crate::chat::turn) fn strip_directives(text: &str) -> String {
    let mut rest = text.trim_start();
    loop {
        if let Some(r) = rest.strip_prefix("Use a canvas for this.") {
            rest = r.trim_start();
        } else if let Some(r) = rest
            .strip_prefix("Use the \"")
            .and_then(|r| r.split_once("\" skill."))
            .map(|(_, r)| r)
        {
            rest = r.trim_start();
        } else if rest.starts_with("Create ")
            && rest.contains(" with generate_image (")
            && rest.contains("\n\n")
        {
            rest = rest.split_once("\n\n").map_or("", |(_, r)| r).trim_start();
        } else if rest.starts_with("> ") {
            rest = rest.split_once("\n\n").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    rest.to_string()
}

/// The text of the first user message.
fn first_message(conversation: &Conversation) -> String {
    conversation
        .messages
        .iter()
        .find_map(|m| match m {
            Message::User { content } => content.iter().find_map(|c| match c {
                rig_core::completion::message::UserContent::Text(t) => Some(t.text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
}

/// The start of the first answer, which often says what the conversation is about when the
/// message itself opens with instructions.
fn first_answer(conversation: &Conversation) -> String {
    conversation
        .messages
        .iter()
        .find_map(|m| match m {
            Message::Assistant(a) => a.content.iter().find_map(|c| match c {
                rig_core::completion::message::AssistantContent::Text(t) => Some(t.text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default()
}

/// The first user message, cut to a sidebar-sized title.
pub(in crate::chat) fn fallback_title(conversation: &Conversation) -> String {
    let first = first_message(conversation);
    let line = strip_directives(&first)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if line.chars().count() > 48 {
        format!("{}…", crate::support::text::clip(&line, 47).trim_end())
    } else if line.is_empty() {
        "New chat".into()
    } else {
        line
    }
}

/// Asks the same model for a short title, falling back to the first message.
pub(super) async fn name_conversation(
    host: Arc<dyn Host>,
    reference: String,
    mut conversation: Conversation,
) {
    let Ok(config_dir) = host.config_dir() else {
        return;
    };
    let mut title = fallback_title(&conversation);
    // The whole opening, not the clipped fallback: a message that starts with instructions
    // ("Answer briefly, without tools: …") has its subject after them.
    let message =
        crate::support::text::clip(strip_directives(&first_message(&conversation)).trim(), 600);
    let answer = crate::support::text::clip(first_answer(&conversation).trim(), 300);
    let prompt = format!(
        "Write a title of at most 6 words for this conversation, about its subject, in the \
         message's language. Reply with the title only, no quotes.\n\nMessage:\n{message}\n\nStart of the answer:\n{answer}"
    );
    // Room for models that think before they answer; the title itself is a few words.
    let request = CompletionRequest::new(Message::user(prompt)).max_tokens(600u64);
    if let Ok(text) = ask_text(&config_dir, &reference, request, 20).await {
        let text = text.trim().trim_matches(['"', '\'', '*', '#']).trim();
        if !text.is_empty() && text.chars().count() <= 80 {
            title = text.to_string();
        }
    }
    // The conversation may have moved on while the title was written; only the title changes.
    let mut latest = store::load_conversation(&config_dir, &conversation.id).unwrap_or_else(|_| {
        conversation.title.clone_from(&title);
        conversation.clone()
    });
    // Untitled, or still the first message standing in while the answer ran: the user has not
    // renamed it, so the model's title takes its place.
    if latest.title.is_empty() || latest.title == fallback_title(&latest) {
        latest.title.clone_from(&title);
        if store::save_conversation(&config_dir, &latest).is_ok() {
            host.emit(ChatEvent::Title {
                conversation_id: latest.id,
                title,
            });
        }
    }
}
