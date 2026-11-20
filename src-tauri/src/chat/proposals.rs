//! `propose_change`: the evolution loop's "propose" step. When the assistant finds a gap (a fact
//! it keeps missing, a procedure it keeps redoing, an instruction that would help) it proposes the
//! smallest change that fixes it, at the cheapest layer, with its evidence. A reviewer that did
//! not write the proposal reads it first; then the user decides. Nothing changes without them.
//!
//! The layers, cheapest first, as the kernel orders them: a memory, a skill, a standing
//! instruction. A new tool has its own path (`create_tool`, `promote_tool`).

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::chat::tools::ChatTool;

pub const PROPOSE_CHANGE: &str = "propose_change";

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "layer", rename_all = "snake_case")]
pub enum Change {
    /// A fact to remember.
    Memory { fact: String },
    /// A written procedure.
    Skill {
        name: String,
        description: String,
        instructions: String,
    },
    /// A line added to the user's standing instructions.
    Instructions { text: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Proposal {
    pub summary: String,
    pub evidence: String,
    pub change: Change,
}

pub fn parse(args: &Value) -> Result<Proposal, String> {
    let proposal: Proposal = serde_json::from_value(args.clone())
        .map_err(|e| format!("The proposal is not described right: {e}"))?;
    if proposal.summary.trim().is_empty() || proposal.evidence.trim().is_empty() {
        return Err("Say what the change is and what evidence calls for it.".into());
    }
    Ok(proposal)
}

/// The instruction for the reviewer, which sees only the proposal, not the chat that wrote it.
pub fn review_prompt(p: &Proposal, original: &Value) -> String {
    format!(
        "You review a change an AI assistant proposes to its own memory, skills or instructions. \
         You did not write it. Check that the evidence supports it, that it is the smallest change \
         that would help, that a cheaper layer (memory before skill before instruction) would not \
         do, and that it holds no secrets or anything the user would not want kept.\n\n\
         Summary: {}\nEvidence: {}\nChange: {}\n\n\
         Reply with one line: SUPPORT: and a reason, or DOUBT: and a reason, under 25 words.",
        p.summary,
        p.evidence,
        original.get("change").cloned().unwrap_or(Value::Null)
    )
}

/// Applies an approved change.
pub fn apply(config_dir: &Path, conversation: &str, change: &Change) -> Result<String, String> {
    match change {
        Change::Memory { fact } => {
            let mut memories = crate::chat::memory::load(config_dir);
            // The same write-time filter as `remember`: sensitive data stays out.
            if let Some(reason) = crate::chat::memory::refuse_sensitive(&memories, fact) {
                return Err(reason);
            }
            let saved = crate::chat::memory::add_filed(
                &mut memories,
                fact,
                "assistant",
                crate::chat::memory::Filing {
                    conversation: Some(conversation.to_string()),
                    ..Default::default()
                },
            )?;
            crate::chat::memory::save(config_dir, &memories)?;
            Ok(format!("Remembered: {}", saved.text))
        }
        Change::Skill {
            name,
            description,
            instructions,
        } => {
            let provenance =
                crate::chat::builtins::skill_provenance(config_dir, name, conversation);
            crate::chat::skills::save(config_dir, name, description, instructions)?;
            crate::chat::traces::register(
                config_dir,
                "skill",
                name,
                Some(&crate::chat::traces::hash(instructions)),
                Some(&provenance),
                "active",
            );
            Ok(format!("Saved the skill `{name}`."))
        }
        Change::Instructions { text } => {
            let mut instructions = crate::chat::store::load_instructions(config_dir);
            let line = text.trim();
            if line.is_empty() {
                return Err("The instruction is empty.".into());
            }
            if !instructions.custom.trim().is_empty() {
                instructions.custom.push('\n');
            }
            instructions.custom.push_str(line);
            crate::chat::store::save_instructions(config_dir, &instructions)?;
            Ok("Added to your instructions.".into())
        }
    }
}

pub fn definition() -> ChatTool {
    ChatTool {
        name: PROPOSE_CHANGE.into(),
        description: "Propose a change to how you work, when you notice a gap that will recur: a \
            fact you keep needing (memory), a procedure you keep redoing (skill), or a standing \
            instruction that would help (instructions). Choose the cheapest layer that fixes it, \
            and give the evidence (what went wrong or was repeated, and where). A reviewer reads \
            it, then the user approves or declines; nothing changes before that."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "The change, in one sentence." },
                "evidence": { "type": "string", "description": "What happened that calls for it." },
                "change": {
                    "type": "object",
                    "description": "One of: {\"layer\":\"memory\",\"fact\":…}, {\"layer\":\"skill\",\"name\":…,\"description\":…,\"instructions\":…}, {\"layer\":\"instructions\",\"text\":…}.",
                    "properties": {
                        "layer": { "type": "string", "enum": ["memory", "skill", "instructions"] },
                        "fact": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "instructions": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["layer"]
                }
            },
            "required": ["summary", "evidence", "change"]
        }),
        connector_id: None,
        tool_name: PROPOSE_CHANGE.into(),
        read_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposals_are_read_and_applied_at_their_layer() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let p = parse(&json!({
            "summary": "Remember the user's time zone",
            "evidence": "Asked twice this week",
            "change": { "layer": "memory", "fact": "Works in Lisbon time" }
        }))
        .unwrap();
        apply(d, "chat-1", &p.change).unwrap();
        assert!(crate::chat::memory::load(d)
            .items
            .iter()
            .any(|m| m.text == "Works in Lisbon time"));
        // A proposed fact passes the same write-time filter as `remember`.
        let secret = Change::Memory {
            fact: "The router password is hunter22-blue".into(),
        };
        assert!(apply(d, "chat-1", &secret)
            .unwrap_err()
            .contains("a password, key or token"));

        // An update to a skill the user wrote keeps it the user's, so the lifecycle never
        // retires it as the assistant's.
        crate::chat::skills::save(d, "my-report", "Use for my report.", "1. Mine.").unwrap();
        apply(
            d,
            "chat-1",
            &Change::Skill {
                name: "my-report".into(),
                description: "Use for my report.".into(),
                instructions: "1. Mine, improved.".into(),
            },
        )
        .unwrap();
        assert_eq!(
            crate::chat::traces::artifact(d, "skill", "my-report")
                .unwrap()
                .provenance
                .as_deref(),
            Some("user")
        );

        let p = parse(&json!({
            "summary": "Prefer metric",
            "evidence": "Corrected three times",
            "change": { "layer": "instructions", "text": "Use metric units." }
        }))
        .unwrap();
        apply(d, "chat-1", &p.change).unwrap();
        assert!(crate::chat::store::load_instructions(d)
            .custom
            .contains("Use metric units."));
        // The instructions as they were are kept.
        assert!(!crate::chat::versions::list(d, "instructions", "user")
            .unwrap()
            .is_empty());

        assert!(parse(
            &json!({ "summary": "", "evidence": "x", "change": { "layer": "memory", "fact": "y" } })
        )
        .is_err());
        assert!(parse(
            &json!({ "summary": "x", "evidence": "y", "change": { "layer": "weights" } })
        )
        .is_err());
    }
}
