//! Learning from a finished turn: a procedure that worked may become a skill.

use super::*;

/// Skills from work (learning.rs): after an agent's turn with several tool steps, a reflection
/// may turn the procedure into a skill, saved turned off; the chat says so.
pub(super) async fn learn_from_turn(host: &dyn Host, id: &str) {
    let Ok(dir) = host.config_dir() else { return };
    if !crate::chat::learning::load(&dir).skills_from_work {
        return;
    }
    let Ok(c) = store::load_conversation(&dir, id) else {
        return;
    };
    let Some(agent) = c.agent_id.as_deref().and_then(|a| agents::find(&dir, a)) else {
        return;
    };
    let turns = store::turns(&c);
    let Some(last) = turns.iter().rev().find(|t| t.role == "assistant") else {
        return;
    };
    let steps: Vec<String> = last
        .parts
        .iter()
        .filter_map(|p| match p {
            store::Part::Tool {
                name, arguments, ..
            } => Some(format!(
                "{name}: {}",
                crate::support::text::clip(arguments.to_string(), 160)
            )),
            _ => None,
        })
        .collect();
    if steps.len() < crate::chat::learning::MIN_STEPS {
        return;
    }
    let request = turns
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map(|t| {
            t.parts
                .iter()
                .filter_map(|p| {
                    if let store::Part::Text { text } = p {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let answer: String = last
        .parts
        .iter()
        .filter_map(|p| {
            if let store::Part::Text { text } = p {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let existing: Vec<String> = crate::chat::skills::list(&dir)
        .into_iter()
        .map(|s| s.name)
        .collect();
    let reference = crate::chat::router::models(&providers::load(&dir))
        .map(|(fast, _)| fast)
        .or_else(|| c.model.clone())
        .or(providers::load(&dir).default_model);
    let Some(reference) = reference else { return };
    let prompt =
        crate::chat::learning::reflect_prompt(&agent.name, &request, &steps, &answer, &existing);
    let reply = match ask_text(
        &dir,
        &reference,
        CompletionRequest::new(Message::user(prompt)),
        60,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("could not reflect on {}'s work: {e}", agent.name);
            return;
        }
    };
    let Some(skill) = crate::chat::learning::parse_reflection(&reply, &existing) else {
        return;
    };
    if let Err(e) = crate::chat::learning::keep(&dir, &agent.name, &skill) {
        warn!("could not keep a learned skill: {e}");
        return;
    }
    info!("{} learned the skill {}", agent.name, skill.name);
    let note = format!(
        "{} learned a skill from this work: /{}, turned off until you review it under Skills.",
        agent.name, skill.name
    );
    if let Ok(mut c) = store::load_conversation(&dir, id) {
        c.notices.push((c.messages.len(), note.clone()));
        if let Err(e) = store::save_conversation(&dir, &c) {
            log::warn!("could not save a notice to conversation {id}: {e}");
        }
    }
    host.emit(ChatEvent::Notice {
        conversation_id: id.into(),
        text: note,
    });
}
