//! The base of the system prompt: tone, time, the user, project, skills, tools and the agent.

use super::*;

pub(super) const TONES: &[(&str, &str)] = &[
    ("formal", "Write in a polite, professional register."),
    ("casual", "Write casually, like a friendly colleague."),
    (
        "empathetic",
        "Be warm and empathetic; acknowledge feelings before advising.",
    ),
    (
        "objective",
        "Be objective and neutral; stick to facts and trade-offs.",
    ),
    ("concise", "Be brief: lead with the answer and skip filler."),
    ("witty", "Keep a light, good-humoured tone where it fits."),
    ("motivational", "Be encouraging and energising."),
    (
        "reflective",
        "Be thoughtful; surface assumptions and second-order effects.",
    ),
];

/// The user's date and time zone, so "today" and deadlines mean what they mean to the user
/// rather than in UTC. No clock time: the preamble then stays the same all day, which keeps
/// provider prompt caches warm.
pub(in crate::chat::turn) fn local_time_line<Tz: chrono::TimeZone>(
    now: chrono::DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let zone = iana_time_zone::get_timezone()
        .ok()
        .filter(|z| !z.is_empty());
    let offset = now.format("%:z").to_string();
    let offset = if offset == "+00:00" {
        "UTC".to_string()
    } else {
        format!("UTC{offset}")
    };
    format!(
        "Today is {} in the user's time zone ({}).",
        now.format("%A, %B %-d, %Y"),
        zone.map_or(offset.clone(), |z| format!("{z}, {offset}"))
    )
}

/// The system prompt: who the assistant is, today's date, and the user's standing instructions.
/// What the system prompt is built from, beside the tools on offer.
#[derive(Default)]
pub struct Context<'a> {
    pub instructions: Option<&'a Instructions>,
    pub project: Option<&'a Project>,
    pub workdir: Option<&'a str>,
    pub memories: Option<&'a memory::Memories>,
    pub skills: &'a [skills::Skill],
    pub canvases: &'a [canvas::Canvas],
    pub agent: Option<&'a agents::Agent>,
    pub libraries: &'a [libraries::Library],
    /// Summaries of the project's other conversations, when the project shares them.
    pub project_digest: Option<String>,
    /// The user's latest message, to pick what fits it when there is too much to list.
    pub query: &'a str,
}

pub(in crate::chat) fn preamble(context: &Context<'_>, tools: &[ChatTool]) -> String {
    preamble_prompt(context, tools).render()
}

/// The system prompt's base, as named sections (see [`crate::chat::prompt`]).
pub(in crate::chat::turn) fn preamble_prompt(
    context: &Context<'_>,
    tools: &[ChatTool],
) -> crate::chat::prompt::Prompt {
    use crate::chat::prompt::{Prompt, ESSENTIAL, OPTIONAL, USEFUL};
    let default_instructions = Instructions::default();
    let instructions = context.instructions.unwrap_or(&default_instructions);
    let project = context.project;
    let workdir = context.workdir;
    let mut p = Prompt::new();
    p.push("identity", ESSENTIAL, identity(context, instructions));
    if !instructions.custom.trim().is_empty() {
        p.push(
            "your instructions",
            ESSENTIAL,
            format!(
                "\n\nThe user's instructions:\n{}",
                instructions.custom.trim()
            ),
        );
    }
    if let Some(project) = project {
        p.push(
            "project",
            ESSENTIAL,
            project_text(project, context.libraries),
        );
        if let Some(digest) = &context.project_digest {
            p.push(
                "project's other chats",
                OPTIONAL,
                format!("\nOther conversations in this project, most recent first (build on them when relevant):\n{digest}"),
            );
        }
    }
    if let Some(dir) = workdir {
        p.push("code session", ESSENTIAL, crate::chat::code::prompt(dir));
    }
    p.push("how to use tools", USEFUL, tool_guidance(tools));
    if let Some(memories) = context.memories.and_then(|m| {
        memory::prompt_for(
            m,
            context.query,
            project.map(|p| p.id.as_str()),
            context.agent.map(|a| a.id.as_str()),
        )
    }) {
        p.push("memories", USEFUL, memories);
    }
    if let Some(skills) = skills::prompt_for(context.skills, context.query) {
        p.push("skills", USEFUL, skills);
    }
    if let Some(canvases) = canvas::prompt(context.canvases) {
        p.push("canvases", OPTIONAL, canvases);
    }
    if tools.iter().any(|t| t.name == builtins::SEARCH_LIBRARY) {
        if let Some(libraries) = libraries::prompt(context.libraries) {
            p.push("libraries", USEFUL, libraries);
        }
    }
    p
}

/// Who the assistant is: the agent's persona and material when one answers, the base persona,
/// the time, the user's name and the tone.
fn identity(context: &Context<'_>, instructions: &Instructions) -> String {
    let project = context.project;
    let mut identity = String::new();
    if let Some(agent) = context.agent {
        identity.push_str(&agent_identity(agent, context.libraries));
    }
    identity.push_str(
        "You are a helpful, accurate assistant running in a desktop app. Answer in the user's \
         language. Use Markdown: short paragraphs, lists and tables where they help, fenced code \
         blocks with a language tag. When you write a text for the user to send or publish (an \
         email, a message, a post, a letter), put that text alone between a `:::writing` line and a \
         `:::` line, so they can copy it. When you are unsure, say so instead of guessing.",
    );
    identity.push_str(&format!("\n\n{}", local_time_line(chrono::Local::now())));
    if !instructions.name.trim().is_empty() {
        identity.push_str(&format!(
            " The user's name is {}.",
            instructions.name.trim()
        ));
    }
    let tone_id = project
        .map(|p| p.tone.as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or(&instructions.tone);
    if let Some((_, tone)) = TONES.iter().find(|(id, _)| *id == tone_id) {
        identity.push_str("\n\n");
        identity.push_str(tone);
    }
    identity
}

/// An agent's persona, role and instructions, and the libraries it searches.
fn agent_identity(agent: &agents::Agent, libraries: &[libraries::Library]) -> String {
    let mut identity = String::new();
    identity.push_str(&format!("You are {}", agent.label()));
    if !agent.description.trim().is_empty() {
        identity.push_str(&format!(": {}", agent.description.trim()));
    }
    identity.push_str("\n\n");
    identity.push_str(&agent.role_prompt());
    if !agent.instructions.trim().is_empty() {
        identity.push_str(agent.instructions.trim());
        identity.push_str("\n\n");
    }
    let own: Vec<&libraries::Library> = agent
        .libraries
        .iter()
        .filter_map(|id| libraries.iter().find(|l| &l.id == id))
        .filter(|l| !l.documents.is_empty())
        .collect();
    if !own.is_empty() {
        let names: Vec<String> = own
            .iter()
            .map(|l| format!("\"{}\" ({} documents)", l.name, l.documents.len()))
            .collect();
        identity.push_str(&format!(
            "Your reference material is in the {} {}; search it with `search_library` before answering questions it may cover, and cite what you use.\n\n",
            if own.len() == 1 { "library" } else { "libraries" },
            names.join(", ")
        ));
    }

    identity
}

/// The project's name and instructions, and its library when it has one.
fn project_text(project: &Project, libraries: &[libraries::Library]) -> String {
    let mut text = format!(
        "\n\nThis conversation belongs to the project \"{}\".",
        project.name
    );
    if !project.instructions.trim().is_empty() {
        text.push_str(" Project instructions:\n");
        text.push_str(project.instructions.trim());
    }
    if let Some(library) = project
        .library_id
        .as_ref()
        .and_then(|id| libraries.iter().find(|l| &l.id == id))
    {
        if !library.documents.is_empty() {
            text.push_str(&format!(
                "\nThe project's files are in the library \"{}\" ({} documents); search it with `search_library` before answering questions they may cover.",
                library.name,
                library.documents.len()
            ));
        }
    }
    text
}

/// How to use the tools the turn has.
fn tool_guidance(tools: &[ChatTool]) -> String {
    let mut guidance = String::new();
    if tools.iter().any(|t| t.name == builtins::WEB_SEARCH) {
        guidance.push_str(
            "\n\nWhen an answer depends on current or specific facts, search the web first. Cite \
             what you use inline as Markdown links whose text is the site's name, like \
             ([rust-lang.org](https://www.rust-lang.org/)), right after the claim they support.",
        );
    }
    if tools.iter().any(|t| t.connector_id.is_some()) {
        guidance.push_str(
            "\n\nYou can use tools on the user's computer and services. Use them when they help, \
             explain briefly what you are doing, and never claim a tool ran when it did not. \
             The user may decline a call; respect that.",
        );
    }
    if tools.iter().any(|t| t.name == builtins::DELEGATE) {
        guidance.push_str(
            "\n\nFor work with several steps, show a plan with `update_plan` first and keep it \
             current. When a task splits into independent parts (several topics, sources or \
             files), hand them to `delegate` so sub-agents research them in parallel, then \
             combine their reports. Call independent tools in the same step; they run at once.",
        );
    }
    guidance
}
