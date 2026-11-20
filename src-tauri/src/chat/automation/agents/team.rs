//! Agents together: the team and group prompts, messages between agents, shared files and routing a message to the right agent.

use super::*;
use crate::chat::error::ChatError;

/// The agents another agent (or the assistant) can hand tasks to.
pub fn team_prompt(agents: &[Agent], except: Option<&str>) -> Option<String> {
    let team: Vec<String> = agents
        .iter()
        .filter(|a| Some(a.id.as_str()) != except)
        .map(|a| {
            format!(
                "- **{}**: {}",
                a.label(),
                if a.description.trim().is_empty() {
                    "no description"
                } else {
                    a.description.trim()
                }
            )
        })
        .collect();
    if team.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n## Your team\nThese agents can take tasks through `delegate`: set a task's `agent` to the \
         agent's name and it works on it with its own instructions and model, then reports back.\n{}",
        team.join("\n")
    ))
}

/// The tool an agent (or the assistant) uses to write to another agent (OpenBot's messages
/// between agents): the other agent answers in a chat of its own, which the user can open.
pub const MESSAGE_AGENT: &str = "message_agent";
/// Messages in flight inside messages: an agent writing to one that writes back stops here.
pub const MAX_MESSAGE_DEPTH: usize = 2;
/// Characters of each shared file passed along.
pub(super) const SHARED_FILE_CHARS: usize = 60_000;

/// What the receiving agent gets: who wrote, the message, and the shared files' text.
pub fn message_text(from: &str, message: &str, files: &[(String, String)]) -> String {
    let mut out = format!("Message from {from}:\n\n{}", message.trim());
    for (name, text) in files {
        let text: String = crate::support::text::clip(text, SHARED_FILE_CHARS);
        out.push_str(&format!(
            "\n\n<attachment name=\"{name}\">\n{text}\n</attachment>"
        ));
    }
    out.push_str("\n\nReply to them directly; your reply is passed back.");
    out
}

/// Reads the files an agent shares: paths inside its folder (or absolute), text only.
pub fn read_shared(
    paths: &[String],
    workdir: Option<&Path>,
) -> Result<Vec<(String, String)>, ChatError> {
    let mut out = Vec::new();
    for p in paths.iter().take(5) {
        let path = Path::new(p);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workdir
                .ok_or_else(|| {
                    ChatError::Invalid(
                        "Relative paths need the chat's folder; give a full path.".into(),
                    )
                })?
                .join(path)
        };
        let bytes = std::fs::read(&full)
            .map_err(|e| ChatError::Invalid(format!("could not read {p}: {e}")))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| ChatError::Invalid(format!("{p} is not a text file.")))?;
        let name = full
            .file_name()
            .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
        out.push((name, text));
    }
    Ok(out)
}

/// The agent's own files: a library named for it, made and given to it on first use.
pub fn files_library(
    config_dir: &Path,
    id: &str,
) -> Result<crate::chat::libraries::Library, ChatError> {
    let mut agent =
        find(config_dir, id).ok_or_else(|| ChatError::Invalid("No such agent.".into()))?;
    let all = crate::chat::libraries::list(config_dir);
    let wanted = format!("{} files", agent.name);
    if let Some(l) = all
        .iter()
        .find(|l| agent.libraries.contains(&l.id) && l.name == wanted)
        .or_else(|| all.iter().find(|l| agent.libraries.contains(&l.id)))
    {
        return Ok(l.clone());
    }
    let library = crate::chat::libraries::create(
        config_dir,
        &wanted,
        &format!(
            "Context for the agent {}: it searches these before answering.",
            agent.name
        ),
    )?;
    agent.libraries.push(library.id.clone());
    save(config_dir, agent)?;
    Ok(library)
}

/// Whether new chats are handed to the best-suited agent (Osaurus's orchestrator).
pub fn routing_enabled(config_dir: &Path) -> bool {
    crate::chat::at_rest::read_to_string(chat_dir(config_dir).join("agent-routing.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

pub fn set_routing(config_dir: &Path, enabled: bool) -> Result<(), ChatError> {
    crate::chat::store::write_json(
        &chat_dir(config_dir).join("agent-routing.json"),
        &serde_json::json!({ "enabled": enabled }),
    )
    .map_err(String::from)
    .map_err(ChatError::Invalid)
}

/// The question that picks an agent for a new chat.
pub fn route_prompt(agents: &[Agent], message: &str) -> String {
    let list: Vec<String> = agents
        .iter()
        .map(|a| {
            format!(
                "- {}: {}",
                a.label(),
                if a.description.trim().is_empty() {
                    "no description"
                } else {
                    a.description.trim()
                }
            )
        })
        .collect();
    format!(
        "A user starts a chat with this message:\n\n{}\n\nThese agents could take it:\n{}\n\nWhich one is clearly \
         the best fit? Reply with only its name, exactly as written, or with `none` when the message \
         is general, small talk, or no agent fits better than a general assistant.",
        crate::support::text::clip(message, 2_000),
        list.join("\n")
    )
}

/// The agent a routing reply names, if any.
pub fn parse_route<'a>(agents: &'a [Agent], reply: &str) -> Option<&'a Agent> {
    let line = reply
        .trim()
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches(['`', '*', '"', '\'', '.'])
        .trim();
    if line.eq_ignore_ascii_case("none") || line.is_empty() {
        return None;
    }
    by_name(agents, line).or_else(|| {
        agents
            .iter()
            .filter(|a| line.to_lowercase().contains(&a.name.to_lowercase()))
            .max_by_key(|a| a.name.len())
    })
}

/// What a group's host is told: its members, and how to run them.
pub fn group_prompt(agents: &[Agent], group: &Agent) -> Option<String> {
    let members: Vec<&Agent> = group
        .members
        .iter()
        .filter_map(|id| agents.iter().find(|a| &a.id == id && a.id != group.id))
        .collect();
    if members.is_empty() {
        return None;
    }
    let mode = match group.group_mode.as_str() {
        "sequential" => "sequential",
        "debate" => "debate",
        _ => "parallel",
    };
    let how = match mode {
        "sequential" => "one after another, each building on the reports before it, in the order listed",
        "debate" => "all at once, then a second round where each answers the others; say where they converged and where they still disagree",
        _ => "all at once, each on its own",
    };
    let list: Vec<String> = members
        .iter()
        .map(|a| {
            format!(
                "- **{}**: {}",
                a.label(),
                if a.description.trim().is_empty() {
                    "no description"
                } else {
                    a.description.trim()
                }
            )
        })
        .collect();
    Some(format!(
        "\n\n## Your group\nYou host a group of agents. For each request, unless it is small talk or a \
         question about the group itself, call `delegate` once with `mode: \"{mode}\"` and one task per \
         member below, in this order, with `agent` set to the member's name. Each task restates the \
         request and what that member should contribute, so it stands alone. The members work {how}. \
         Then answer from their reports: name who said what, show where they agree and differ, and \
         end with your conclusion.\n{}",
        list.join("\n")
    ))
}

/// Finds an agent by name (any case) or id; a label with its title (`João [3D Artist]`) works too.
pub fn by_name<'a>(agents: &'a [Agent], name: &str) -> Option<&'a Agent> {
    let name = name.trim();
    let bare = match name.rsplit_once(" [") {
        Some((n, rest)) if rest.ends_with(']') => n.trim(),
        _ => name,
    };
    agents.iter().find(|a| {
        a.id == name || a.name.eq_ignore_ascii_case(name) || a.name.eq_ignore_ascii_case(bare)
    })
}
