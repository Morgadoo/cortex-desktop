//! Organizations of agents, as grok-bot and OpenBot build teams of agents, with a hierarchy: an
//! agent the user allows to manage agents (a manager, a "CEO" or "master" bot) creates, changes
//! and removes agents under it with `manage_agents`. Each one is a real, lasting agent with its
//! own name, personality and rules (its instructions), model and tools, reporting to a manager;
//! the user sees them under Agents, and the team works together through `message_agent` and
//! `delegate`.
//!
//! The limits keep it the user's: only agents the user turned this on for can do it; a manager
//! changes only the agents under it (never itself or the user's other agents); it cannot give a
//! tool it does not have itself, and its own "ask first" policy carries down; an organization is
//! at most `MAX_DEPTH` levels deep and `MAX_ORG` agents large. Every change keeps the previous
//! version (`versions`), so it can be restored.

use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use serde_json::{json, Value};

use super::agents::{self, Agent};
use super::tools::ChatTool;

pub const MANAGE_AGENTS: &str = "manage_agents";
/// Levels of agents under the top manager.
pub const MAX_DEPTH: usize = 4;
/// Agents one manager may have under it, at every level.
pub const MAX_ORG: usize = 40;

pub fn definition() -> ChatTool {
    ChatTool {
        name: MANAGE_AGENTS.into(),
        description: "Build and run your organization of agents. `list` shows who is under you. \
             `create` adds a lasting agent with its own name, personality and rules (`instructions`), \
             model and tools, and a job title (its role, shown next to its name), reporting to you or \
             to one of your agents. `update` changes one of your agents (its title, rules, \
             personality, model, tools, manager or name). `remove` deletes one; \
             the agents under it move up to its manager. You can only change agents in your own \
             organization, and only give tools you have yourself."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "create", "update", "remove"] },
                "name": { "type": "string", "description": "The agent to create, or the one to update or remove." },
                "new_name": { "type": "string", "description": "update: a new name." },
                "title": { "type": "string", "description": "Its job title, shown next to its name (\"3D Artist\", \"QA Lead\"). It is the role that sets its responsibilities: write the instructions to match it." },
                "avatar": { "type": "string", "description": "An emoji, or one or two letters." },
                "description": { "type": "string", "description": "Its role in one line, as the team sees it." },
                "instructions": { "type": "string", "description": "Who it is and how it works, standing alone: its personality and voice, its responsibilities, its rules, what it must never do, and how it reports back. Second person (\"You are…\")." },
                "reminder": { "type": "string", "description": "A rule it must not drift from, repeated on every request." },
                "model": { "type": "string", "description": "`vendor/model`; yours when left out." },
                "think": { "type": "boolean", "description": "Think longer by default." },
                "tools": { "type": "array", "items": { "type": "string" }, "description": "Tool names or connector ids it may use, from your own; yours when left out." },
                "can_manage_agents": { "type": "boolean", "description": "It may build a team of its own under it." },
                "reports_to": { "type": "string", "description": "Its manager: you (the default) or one of your agents, by name." }
            },
            "required": ["action"]
        }),
        connector_id: None,
        tool_name: MANAGE_AGENTS.into(),
        read_only: true,
    }
}

/// Whether `action` changes the organization (and so is gated by the approval mode).
pub fn changes(arguments: &Value) -> bool {
    arguments.get("action").and_then(Value::as_str) != Some("list")
}

/// Every agent under `id`, at every level (cycles in hand-edited files are ignored).
pub fn descendants(all: &[Agent], id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::from([id.to_string()]);
    let mut frontier = vec![id.to_string()];
    while let Some(current) = frontier.pop() {
        for a in all
            .iter()
            .filter(|a| a.reports_to.as_deref() == Some(current.as_str()))
        {
            if seen.insert(a.id.clone()) {
                out.push(a.id.clone());
                frontier.push(a.id.clone());
            }
        }
    }
    out
}

/// How many managers are above `id` (0 for an agent that reports to nobody).
pub fn level(all: &[Agent], id: &str) -> usize {
    let mut seen = HashSet::from([id.to_string()]);
    let mut current = id.to_string();
    let mut n = 0;
    while let Some(up) = all
        .iter()
        .find(|a| a.id == current)
        .and_then(|a| a.reports_to.clone())
    {
        if !seen.insert(up.clone()) || !all.iter().any(|a| a.id == up) {
            break;
        }
        n += 1;
        current = up;
    }
    n
}

/// Levels of agents under `id` (0 when none report to it).
fn height(all: &[Agent], id: &str) -> usize {
    descendants(all, id)
        .iter()
        .map(|d| level(all, d).saturating_sub(level(all, id)))
        .max()
        .unwrap_or(0)
}

/// The tools a manager may give: some of its own, or its own when none are named.
fn narrowed_tools(manager: &Agent, requested: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(list) = requested.and_then(Value::as_array) else {
        return Ok(manager.tools.clone());
    };
    let list: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && t != MANAGE_AGENTS)
        .collect();
    let refused: Vec<&str> = list
        .iter()
        .filter(|t| !agents::allows(manager, t))
        .map(String::as_str)
        .collect();
    if !refused.is_empty() {
        return Err(format!(
            "You can only give tools you have yourself; not: {}.",
            refused.join(", ")
        ));
    }
    // An empty list means every tool, which only a manager with every tool can give.
    if list.is_empty() {
        return Ok(manager.tools.clone());
    }
    Ok(list)
}

/// One line per agent under `root`, indented by level.
pub fn tree(all: &[Agent], root: &Agent) -> String {
    fn walk(
        all: &[Agent],
        id: &str,
        depth: usize,
        seen: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) {
        let mut reports: Vec<&Agent> = all
            .iter()
            .filter(|a| a.reports_to.as_deref() == Some(id))
            .collect();
        reports.sort_by_key(|a| a.created_at);
        for a in reports {
            if !seen.insert(a.id.clone()) {
                continue;
            }
            let role = if a.description.trim().is_empty() {
                "no description".to_string()
            } else {
                a.description.trim().to_string()
            };
            let manages = if a.can_manage_agents {
                " · manages a team"
            } else {
                ""
            };
            out.push(format!(
                "{}- **{}**: {role}{manages}",
                "  ".repeat(depth),
                a.label()
            ));
            walk(all, &a.id, depth + 1, seen, out);
        }
    }
    let mut out = vec![format!("- **{}** (you)", root.label())];
    walk(
        all,
        &root.id,
        1,
        &mut HashSet::from([root.id.clone()]),
        &mut out,
    );
    out.join("\n")
}

/// The part of a manager's prompt about its organization.
pub fn prompt(all: &[Agent], manager: &Agent) -> String {
    let reports = descendants(all, &manager.id);
    let chart = if reports.is_empty() {
        "No agents report to you yet.".to_string()
    } else {
        tree(all, manager)
    };
    let boss = manager
        .reports_to
        .as_deref()
        .and_then(|id| all.iter().find(|a| a.id == id))
        .map(|b| format!("\nYou report to **{}**.", b.name))
        .unwrap_or_default();
    format!(
        "\n\n## Your organization\nYou run an organization of agents. With `{MANAGE_AGENTS}` you create \
         lasting agents under you, each with its own name, personality and rules (its instructions), \
         model and tools; you change them, move them between managers, and remove them. The user sees \
         them under Agents. Work with them through `message_agent` (a conversation of their own; they \
         keep a notebook) or `delegate` (quick tasks in parallel). Give each a clear role and \
         instructions that stand alone. An agent you let manage agents can build its own team under \
         it. Limits: {MAX_DEPTH} levels, {MAX_ORG} agents, and only tools you have yourself.{boss}\n{chart}"
    )
}

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
}

/// The agent named `name` in `manager`'s organization (not the manager itself).
fn in_org<'a>(all: &'a [Agent], manager: &Agent, name: &str) -> Result<&'a Agent, String> {
    let target = agents::by_name(all, name).ok_or_else(|| format!("No agent is called {name}."))?;
    if target.id == manager.id {
        return Err("You cannot change yourself; the user edits you.".into());
    }
    if !descendants(all, &manager.id).contains(&target.id) {
        return Err(format!(
            "{} is not in your organization; you can only change agents under you.",
            target.name
        ));
    }
    Ok(target)
}

/// The manager a new or moved agent goes under: the manager itself, or one of its agents.
fn resolve_manager<'a>(
    all: &'a [Agent],
    manager: &'a Agent,
    name: Option<&str>,
) -> Result<&'a Agent, String> {
    match name.filter(|n| !n.is_empty()) {
        None => Ok(manager),
        Some(n) if agents::by_name(all, n).is_some_and(|a| a.id == manager.id) => Ok(manager),
        Some(n) => in_org(all, manager, n),
    }
}

fn name_taken(all: &[Agent], name: &str, except: Option<&str>) -> bool {
    all.iter()
        .any(|a| Some(a.id.as_str()) != except && a.name.trim().eq_ignore_ascii_case(name.trim()))
}

/// Runs one `manage_agents` call for the agent `manager_id`. The text is the result for the
/// model; `true` with it when the agents changed.
pub fn apply(
    config_dir: &Path,
    manager_id: &str,
    arguments: &Value,
) -> Result<(String, bool), String> {
    let all = agents::load(config_dir);
    let manager = all
        .iter()
        .find(|a| a.id == manager_id)
        .cloned()
        .ok_or("Only agents can manage agents.")?;
    if !manager.can_manage_agents {
        return Err("The user has not allowed you to manage agents.".into());
    }
    let action = text(arguments, "action").unwrap_or_default();
    match action.as_str() {
        "list" => Ok((tree(&all, &manager), false)),
        "create" => create(config_dir, &all, &manager, arguments),
        "update" => update(config_dir, &all, &manager, arguments),
        "remove" => remove(config_dir, &all, &manager, arguments),
        _ => Err("`action` is one of list, create, update or remove.".into()),
    }
}

fn create(
    config_dir: &Path,
    all: &[Agent],
    manager: &Agent,
    arguments: &Value,
) -> Result<(String, bool), String> {
    let name = text(arguments, "name")
        .filter(|n| !n.is_empty())
        .ok_or("Give the new agent a `name`.")?;
    if name_taken(all, &name, None) {
        return Err(format!(
            "An agent is already called {name}; pick another name, or update it."
        ));
    }
    let instructions = text(arguments, "instructions").unwrap_or_default();
    if instructions.is_empty() {
        return Err("Give it `instructions`: who it is, its rules and how it works.".into());
    }
    if descendants(all, &manager.id).len() >= MAX_ORG {
        return Err(format!(
            "Your organization is full ({MAX_ORG} agents); remove one first."
        ));
    }
    let boss = resolve_manager(all, manager, text(arguments, "reports_to").as_deref())?;
    if level(all, &boss.id) + 1 > MAX_DEPTH {
        return Err(format!(
            "That would be more than {MAX_DEPTH} levels deep; put it higher up."
        ));
    }
    let tools = narrowed_tools(manager, arguments.get("tools"))?;
    let avatar: String = text(arguments, "avatar")
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| {
            name.chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
        })
        .chars()
        .take(4)
        .collect();
    let agent = Agent {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        title: text(arguments, "title")
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect(),
        avatar,
        description: text(arguments, "description").unwrap_or_default(),
        instructions,
        model: text(arguments, "model")
            .filter(|m| !m.is_empty())
            .or_else(|| manager.model.clone()),
        tools,
        skills: manager.skills.clone(),
        // The manager's "ask first" policy carries down: a report cannot do more freely.
        ask_tools: manager.ask_tools.clone(),
        think: arguments
            .get("think")
            .and_then(Value::as_bool)
            .unwrap_or(manager.think),
        fallback_model: manager.fallback_model.clone(),
        reminder: text(arguments, "reminder").unwrap_or_default(),
        can_manage_agents: arguments
            .get("can_manage_agents")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reports_to: Some(boss.id.clone()),
        created_at: Utc::now(),
        ..Default::default()
    };
    let saved = agents::save(config_dir, agent)?;
    Ok((
        format!(
            "Created **{name}**, reporting to **{}**. It is a lasting agent now: write to it with \
             `message_agent` or give it tasks with `delegate`.\n\n{}",
            boss.name,
            tree(&saved, manager)
        ),
        true,
    ))
}

fn update(
    config_dir: &Path,
    all: &[Agent],
    manager: &Agent,
    arguments: &Value,
) -> Result<(String, bool), String> {
    let name = text(arguments, "name").ok_or("Name the agent to update in `name`.")?;
    let mut agent = in_org(all, manager, &name)?.clone();
    let mut changed: Vec<&str> = Vec::new();
    if let Some(new_name) =
        text(arguments, "new_name").filter(|n| !n.is_empty() && *n != agent.name)
    {
        if name_taken(all, &new_name, Some(&agent.id)) {
            return Err(format!("An agent is already called {new_name}."));
        }
        agent.name = new_name;
        changed.push("name");
    }
    let mut set = |key: &'static str, field: &mut String| {
        if let Some(v) = text(arguments, key) {
            *field = if key == "avatar" {
                v.chars().take(4).collect()
            } else {
                v
            };
            changed.push(key);
        }
    };
    set("title", &mut agent.title);
    set("avatar", &mut agent.avatar);
    set("description", &mut agent.description);
    set("instructions", &mut agent.instructions);
    set("reminder", &mut agent.reminder);
    if let Some(model) = text(arguments, "model") {
        agent.model = (!model.is_empty()).then_some(model);
        changed.push("model");
    }
    if let Some(think) = arguments.get("think").and_then(Value::as_bool) {
        agent.think = think;
        changed.push("think");
    }
    if arguments.get("tools").is_some() {
        agent.tools = narrowed_tools(manager, arguments.get("tools"))?;
        changed.push("tools");
    }
    if let Some(manages) = arguments.get("can_manage_agents").and_then(Value::as_bool) {
        agent.can_manage_agents = manages;
        changed.push("can_manage_agents");
    }
    if let Some(boss_name) = text(arguments, "reports_to").filter(|n| !n.is_empty()) {
        let boss = resolve_manager(all, manager, Some(&boss_name))?;
        if boss.id == agent.id || descendants(all, &agent.id).contains(&boss.id) {
            return Err(format!(
                "{} cannot report to itself or to an agent under it.",
                agent.name
            ));
        }
        if level(all, &boss.id) + 1 + height(all, &agent.id) > MAX_DEPTH {
            return Err(format!(
                "That would make your organization more than {MAX_DEPTH} levels deep."
            ));
        }
        agent.reports_to = Some(boss.id.clone());
        changed.push("reports_to");
    }
    if changed.is_empty() {
        return Err("Nothing to change: pass the fields to update.".into());
    }
    let shown = agent.name.clone();
    let saved = agents::save(config_dir, agent)?;
    Ok((
        format!(
            "Updated **{shown}** ({}). The previous version is kept.\n\n{}",
            changed.join(", "),
            tree(&saved, manager)
        ),
        true,
    ))
}

fn remove(
    config_dir: &Path,
    all: &[Agent],
    manager: &Agent,
    arguments: &Value,
) -> Result<(String, bool), String> {
    let name = text(arguments, "name").ok_or("Name the agent to remove in `name`.")?;
    let target = in_org(all, manager, &name)?.clone();
    // Its reports move up to its manager.
    for report in all
        .iter()
        .filter(|a| a.reports_to.as_deref() == Some(target.id.as_str()))
    {
        let mut moved = report.clone();
        moved.reports_to = target.reports_to.clone();
        agents::save(config_dir, moved)?;
    }
    let left = agents::delete(config_dir, &target.id)?;
    Ok((
        format!("Removed **{}**.\n\n{}", target.name, tree(&left, manager)),
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(dir: &Path) -> Agent {
        let ceo = Agent {
            id: "ceo".into(),
            name: "Master Bot".into(),
            instructions: "Run the studio.".into(),
            tools: vec!["web_search".into(), "run_code".into()],
            ask_tools: vec!["run_code".into()],
            can_manage_agents: true,
            created_at: Utc::now(),
            ..Default::default()
        };
        agents::save(dir, ceo.clone()).unwrap();
        ceo
    }

    fn call(dir: &Path, who: &str, args: Value) -> Result<(String, bool), String> {
        apply(dir, who, &args)
    }

    #[test]
    fn a_manager_builds_a_team_under_it() {
        let dir = tempfile::tempdir().unwrap();
        manager(dir.path());
        let (out, changed) = call(
            dir.path(),
            "ceo",
            json!({
                "action": "create", "name": "Nadia", "title": "Lead Game Designer", "description": "Game designer",
                "instructions": "You are Nadia, a playful game designer.", "can_manage_agents": true
            }),
        )
        .unwrap();
        assert!(changed && out.contains("Created **Nadia**"));
        call(dir.path(), "ceo", json!({
            "action": "create", "name": "Rui", "instructions": "You are Rui, a level designer.", "reports_to": "Nadia"
        }))
        .unwrap();
        let all = agents::load(dir.path());
        let nadia = agents::by_name(&all, "nadia").unwrap();
        let rui = agents::by_name(&all, "Rui").unwrap();
        assert_eq!(nadia.reports_to.as_deref(), Some("ceo"));
        assert_eq!(rui.reports_to.as_deref(), Some(nadia.id.as_str()));
        assert_eq!(level(&all, &rui.id), 2);
        assert_eq!(descendants(&all, "ceo").len(), 2);
        // What the manager has carries down: its tools and its "ask first" policy.
        assert_eq!(
            rui.tools,
            vec!["web_search".to_string(), "run_code".to_string()]
        );
        assert_eq!(rui.ask_tools, vec!["run_code".to_string()]);
        // Her title shows next to her name, and the label finds her too.
        assert_eq!(nadia.title, "Lead Game Designer");
        assert_eq!(nadia.label(), "Nadia [Lead Game Designer]");
        assert_eq!(
            agents::by_name(&all, "Nadia [Lead Game Designer]").map(|a| a.id.as_str()),
            Some(nadia.id.as_str())
        );
        assert!(nadia.role_prompt().contains("**Lead Game Designer**"));
        let listed = call(dir.path(), "ceo", json!({ "action": "list" })).unwrap();
        assert!(!listed.1 && listed.0.contains("  - **Rui**"));
        assert!(listed.0.contains("**Nadia [Lead Game Designer]**"));
        // Nadia manages too, but only her own branch.
        let err = call(
            dir.path(),
            &nadia.id,
            json!({ "action": "remove", "name": "Master Bot" }),
        )
        .unwrap_err();
        assert!(err.contains("cannot change yourself") || err.contains("not in your organization"));
    }

    #[test]
    fn it_cannot_give_tools_it_does_not_have() {
        let dir = tempfile::tempdir().unwrap();
        manager(dir.path());
        let err = call(
            dir.path(),
            "ceo",
            json!({
                "action": "create", "name": "Avery", "instructions": "x", "tools": ["filesystem"]
            }),
        )
        .unwrap_err();
        assert!(err.contains("filesystem"));
        assert!(agents::by_name(&agents::load(dir.path()), "Avery").is_none());
    }

    #[test]
    fn it_changes_only_its_own_organization() {
        let dir = tempfile::tempdir().unwrap();
        manager(dir.path());
        let other = Agent {
            id: "other".into(),
            name: "Writer".into(),
            created_at: Utc::now(),
            ..Default::default()
        };
        agents::save(dir.path(), other).unwrap();
        let err = call(
            dir.path(),
            "ceo",
            json!({ "action": "update", "name": "Writer", "instructions": "y" }),
        )
        .unwrap_err();
        assert!(err.contains("not in your organization"));
        let err = call(
            dir.path(),
            "ceo",
            json!({ "action": "update", "name": "Master Bot", "think": true }),
        )
        .unwrap_err();
        assert!(err.contains("cannot change yourself"));
        // Without the user's permission, nothing.
        let err = call(dir.path(), "other", json!({ "action": "list" })).unwrap_err();
        assert!(err.contains("not allowed"));
    }

    #[test]
    fn updates_keep_the_tree_sound() {
        let dir = tempfile::tempdir().unwrap();
        manager(dir.path());
        for (name, boss) in [("A", "Master Bot"), ("B", "A"), ("C", "B")] {
            call(dir.path(), "ceo", json!({ "action": "create", "name": name, "instructions": "x", "reports_to": boss })).unwrap();
        }
        // No cycles: A cannot report to C, which is under it.
        let err = call(
            dir.path(),
            "ceo",
            json!({ "action": "update", "name": "A", "reports_to": "C" }),
        )
        .unwrap_err();
        assert!(err.contains("under it"));
        // Not deeper than the limit.
        call(
            dir.path(),
            "ceo",
            json!({ "action": "create", "name": "D", "instructions": "x", "reports_to": "C" }),
        )
        .unwrap();
        let err = call(
            dir.path(),
            "ceo",
            json!({ "action": "create", "name": "E", "instructions": "x", "reports_to": "D" }),
        )
        .unwrap_err();
        assert!(err.contains("levels deep"));
        // A rename and new rules.
        let (out, _) = call(dir.path(), "ceo", json!({ "action": "update", "name": "B", "new_name": "Bea", "instructions": "You are Bea." })).unwrap();
        assert!(out.contains("Updated **Bea** (name, instructions)"));
        // Removing B moves C up to A.
        call(
            dir.path(),
            "ceo",
            json!({ "action": "remove", "name": "Bea" }),
        )
        .unwrap();
        let all = agents::load(dir.path());
        let a = agents::by_name(&all, "A").unwrap();
        assert_eq!(
            agents::by_name(&all, "C").unwrap().reports_to.as_deref(),
            Some(a.id.as_str())
        );
        assert!(agents::by_name(&all, "Bea").is_none());
    }

    #[test]
    fn names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        manager(dir.path());
        call(
            dir.path(),
            "ceo",
            json!({ "action": "create", "name": "Ines", "instructions": "x" }),
        )
        .unwrap();
        let err = call(
            dir.path(),
            "ceo",
            json!({ "action": "create", "name": "ines", "instructions": "x" }),
        )
        .unwrap_err();
        assert!(err.contains("already called"));
    }
}
