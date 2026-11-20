//! Summarising earlier turns when a conversation nears its model's window.

use super::*;

/// Summarizes the turns before the last few once the conversation nears the model's window.
/// What [`compact_if_needed`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compaction {
    /// The context fits, or there is nothing older than the recent turns to summarize.
    NotNeeded,
    Done,
    Failed,
}

pub(super) async fn compact_if_needed(
    host: &dyn Host,
    model: &DynModel<Completion>,
    conversation: &mut Conversation,
    window: u64,
    config_dir: &std::path::Path,
) -> Compaction {
    let measured = if conversation.usage.context_tokens > 0 {
        conversation.usage.context_tokens
    } else {
        context::estimate_tokens(&context::request_messages(
            &conversation.messages,
            conversation.summary.as_ref(),
        ))
    };
    if !context::should_compact(measured, window) {
        return Compaction::NotNeeded;
    }
    let covered = conversation.summary.as_ref().map_or(0, |s| s.upto);
    let Some(point) = context::compaction_point(&conversation.messages, covered) else {
        return Compaction::NotNeeded;
    };
    let id = conversation.id.clone();
    host.emit(ChatEvent::Compacting {
        conversation_id: id.clone(),
    });
    let transcript = context::transcript(&conversation.messages[covered..point]);
    let prompt = context::summary_prompt(
        conversation.summary.as_ref().map(|s| s.text.as_str()),
        &transcript,
    );
    let request = CompletionRequest::new(Message::user(prompt)).max_tokens(1500u64);
    match tokio::time::timeout(std::time::Duration::from_secs(120), model.call(request)).await {
        Ok(Ok(response)) if !response.text().trim().is_empty() => {
            info!("compacted conversation {id} up to message {point}");
            conversation.usage.add(&response.usage);
            conversation.summary = Some(context::Summary {
                text: response.text().trim().to_string(),
                upto: point,
                created_at: Utc::now(),
            });
            // The next reply measures the smaller context afresh.
            conversation.usage.context_tokens = 0;
            if let Err(e) = store::save_conversation(config_dir, conversation) {
                warn!("could not save compacted conversation: {e}");
            }
            host.emit(ChatEvent::Compacted {
                conversation_id: id,
                upto: point,
            });
            Compaction::Done
        }
        Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
            warn!("compaction failed; continuing with tool-result clearing only");
            host.emit(ChatEvent::Compacted {
                conversation_id: id,
                upto: covered,
            });
            Compaction::Failed
        }
    }
}
