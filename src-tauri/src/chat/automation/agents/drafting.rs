//! Drafting an agent from a sentence, and refining a draft.

use super::*;
use crate::chat::error::ChatError;

/// The instruction that turns a one-line description into an agent.
pub(in crate::chat) fn draft_prompt(
    description: &str,
    tools: &[&str],
    skills: &[String],
) -> String {
    format!(
        "Design an AI agent from this description:\n\n{description}\n\nReply with only a JSON object, \
         no prose and no code fence, with these fields:\n\
         - name: 1 to 3 words\n\
         - title: its job title in 1 to 4 words (\"3D Artist\", \"QA Lead\"): the role that sets its responsibilities\n\
         - avatar: one emoji\n\
         - description: one line saying what it is for\n\
         - instructions: 120 to 250 words, in the second person: who it is, how it works step by \
           step, what good output looks like, and what it must never do\n\
         - tools: the subset of {tools:?} it needs (fewest that do the job)\n\
         - skills: the subset of {skills:?} it should use, or []\n\
         - think: true only for analysis-heavy work"
    )
}

/// The prompt asking a model to change a drafted agent as the user asks, keeping the rest.
pub(in crate::chat) fn refine_prompt(
    current: &Agent,
    request: &str,
    tools: &[&str],
    skills: &[String],
) -> String {
    let now = serde_json::json!({
        "name": current.name, "title": current.title, "avatar": current.avatar, "description": current.description,
        "instructions": current.instructions, "tools": current.tools, "skills": current.skills, "think": current.think,
    });
    format!(
        "Here is an AI agent's definition as JSON:\n{now}\n\nRevise it as the user asks: {request}\n\
         Keep what they did not ask to change. Reply with only the full JSON object, no prose and no \
         code fence, with the same fields; tools must come from {tools:?} and skills from {skills:?}."
    )
}

/// Reads the model's draft. Unknown tools and skills are dropped.
pub(in crate::chat) fn parse_draft(
    text: &str,
    tools: &[&str],
    skills: &[String],
) -> Result<Agent, ChatError> {
    let start = text
        .find('{')
        .ok_or_else(|| ChatError::Invalid("The model did not return an agent.".into()))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| ChatError::Invalid("The model did not return an agent.".into()))?;
    let value: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|_| "The model's draft was not valid JSON; try again.".to_string())
        .map_err(ChatError::Invalid)?;
    let field = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let list = |k: &str| -> Vec<String> {
        value
            .get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let name = field("name");
    if name.is_empty() {
        return Err(ChatError::Invalid(
            "The draft had no name; try again.".into(),
        ));
    }
    Ok(Agent {
        id: uuid::Uuid::new_v4().to_string(),
        title: field("title").chars().take(60).collect(),
        avatar: field("avatar").chars().take(4).collect(),
        description: field("description"),
        instructions: field("instructions"),
        model: None,
        tools: list("tools")
            .into_iter()
            .filter(|t| tools.contains(&t.as_str()))
            .collect(),
        skills: list("skills")
            .into_iter()
            .filter(|s| skills.contains(s))
            .collect(),
        ask_tools: Vec::new(),
        think: value
            .get("think")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        fallback_model: None,
        monthly_budget: None,
        starters: Vec::new(),
        libraries: Vec::new(),
        secrets: Vec::new(),
        reminder: String::new(),
        members: Vec::new(),
        group_mode: String::new(),
        code_network: false,
        can_manage_agents: false,
        reports_to: None,
        created_at: Utc::now(),
        name,
    })
}
