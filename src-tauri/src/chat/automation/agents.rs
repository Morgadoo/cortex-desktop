//! Agents: named assistants with their own persona, model, tools and standing tasks.
//!
//! The workforce: each agent is a reusable configuration a conversation or a scheduled task runs
//! under. Its instructions replace the generic persona, its model overrides the composer's, and
//! its tool list narrows what it may use. Saved in `<config>/chat/agents.json`.

use crate::support::settings_file::SettingsFile;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::chat::store::{chat_dir, valid_id};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Its job title ("3D Artist", "QA Lead"), shown next to its name: the role that sets its
    /// responsibilities in the team.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// An emoji or one or two letters.
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub description: String,
    /// Who the agent is and how it works; replaces the generic assistant persona.
    #[serde(default)]
    pub instructions: String,
    /// `vendor/model`; the composer's model when empty.
    #[serde(default)]
    pub model: Option<String>,
    /// Tool names (`web_search`) or connector ids (`filesystem`) it may use; all when empty.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Skills it may load; all enabled ones when empty.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Tools (names or connector ids) that always ask before running for this agent, whatever
    /// the approval mode: the "Ask" level of a tool policy (allowed / ask / blocked).
    #[serde(default)]
    pub ask_tools: Vec<String>,
    /// Think longer by default.
    #[serde(default)]
    pub think: bool,
    /// The model to switch to when the main one fails (a provider down, out of credit).
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// US dollars this agent may spend in a calendar month; no limit when `None`.
    #[serde(default)]
    pub monthly_budget: Option<f64>,
    /// Prompts offered on the agent's new-chat page (Open WebUI's prompt suggestions).
    #[serde(default)]
    pub starters: Vec<String>,
    /// Libraries the agent searches before answering, by id.
    #[serde(default)]
    pub libraries: Vec<String>,
    /// Vault secrets it may use, by name (besides those marked for everywhere).
    #[serde(default)]
    pub secrets: Vec<String>,
    /// A short reminder added to the newest message on every request (Onyx's per-turn
    /// reminder), for rules the agent must not drift from in long chats.
    #[serde(default)]
    pub reminder: String,
    /// The agents this one hosts as a group (LobeHub's agent groups), by id; empty for a single
    /// agent. A group answers by handing each request to its members, then writing the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// How the members work: `parallel`, `sequential` or `debate`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group_mode: String,
    /// Its code may reach the internet (to install packages into its own environment).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code_network: bool,
    /// It runs an organization (a manager or "CEO" agent): it may create, change and remove the
    /// agents under it with `manage_agents`. Off unless the user turns it on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_manage_agents: bool,
    /// The agent this one reports to, by id: the manager that created it or took it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports_to: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Agent {
    /// Its name with its title, as the team sees it: `João [3D Artist]`.
    pub fn label(&self) -> String {
        if self.title.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{} [{}]", self.name, self.title.trim())
        }
    }

    /// What its title means for how it works, for its own prompt; empty without a title.
    pub fn role_prompt(&self) -> String {
        let title = self.title.trim();
        if title.is_empty() {
            return String::new();
        }
        format!(
            "Your title is **{title}**. It is your role in the team and sets your responsibilities: \
             own what a {title} owns, decide what is yours to decide, hold the standards of the role, \
             and hand what belongs to another role to the agent who holds it.\n\n"
        )
    }
}

const AGENTS_FILE: SettingsFile<Vec<Agent>> = SettingsFile::sealed("chat/agents.json");

pub fn load(config_dir: &Path) -> Vec<Agent> {
    AGENTS_FILE.load(config_dir)
}

/// An agent as one file: settings, notebook and the user's own skills it relies on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    /// `cortex-agent` (`lumo-agent` in files exported by Lumo Desktop).
    pub format: String,
    pub version: u32,
    pub agent: Agent,
    #[serde(default)]
    pub notebook: String,
    #[serde(default)]
    pub skills: Vec<BundledSkill>,
    /// The MCP tool servers the agent's tool list names, without secrets (environment values
    /// blanked; keys stay in the keychain they came from).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<serde_json::Value>,
}

/// The MCP servers an agent names in its tools (`mcp-<id>`), made safe to share: environment
/// values are blanked, since a key may have been typed there instead of the keychain.
pub fn shareable_mcp_servers(
    agent: &Agent,
    servers: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let named = |id: &str| {
        let connector = format!("mcp-{id}");
        agent
            .tools
            .iter()
            .chain(agent.ask_tools.iter())
            .any(|t| *t == connector || t.starts_with(&format!("{connector}__")))
    };
    servers
        .into_iter()
        .filter(|s| {
            s.get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(named)
        })
        .map(|mut s| {
            if let Some(env) = s.get_mut("env").and_then(serde_json::Value::as_object_mut) {
                for value in env.values_mut() {
                    *value = serde_json::Value::String(String::new());
                }
            }
            s
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BundledSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

pub fn bundle(config_dir: &Path, id: &str) -> Result<Bundle, String> {
    let agent = find(config_dir, id).ok_or("There is no such agent.")?;
    // Built-in skills exist everywhere; only the user's own travel with the agent.
    let skills = crate::chat::skills::list(config_dir)
        .into_iter()
        .filter(|s| s.origin == "user" && agent.skills.contains(&s.name))
        .map(|s| BundledSkill {
            name: s.name,
            description: s.description,
            body: s.body,
        })
        .collect();
    Ok(Bundle {
        format: "cortex-agent".into(),
        version: 1,
        notebook: notebook(config_dir, id),
        skills,
        agent,
        mcp_servers: Vec::new(),
    })
}

/// Adds a bundled agent as a new one; skills of the same name already here are kept as they are.
pub fn install(config_dir: &Path, bundle: Bundle) -> Result<Agent, String> {
    // `lumo-agent` files were exported before the rename to Cortex.
    if !matches!(bundle.format.as_str(), "cortex-agent" | "lumo-agent") {
        return Err("This file is not an exported agent.".into());
    }
    let existing = load(config_dir);
    let mut agent = bundle.agent;
    agent.id = uuid::Uuid::new_v4().to_string();
    agent.created_at = Utc::now();
    if existing
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(&agent.name))
    {
        agent.name = format!("{} (imported)", agent.name);
    }
    let have = crate::chat::skills::list(config_dir);
    for skill in bundle
        .skills
        .iter()
        .filter(|s| !have.iter().any(|h| h.name == s.name))
    {
        crate::chat::skills::save(config_dir, &skill.name, &skill.description, &skill.body)?;
    }
    save(config_dir, agent.clone())?;
    if !bundle.notebook.trim().is_empty() {
        save_notebook(config_dir, &agent.id, &bundle.notebook)?;
    }
    Ok(agent)
}

pub fn find(config_dir: &Path, id: &str) -> Option<Agent> {
    load(config_dir).into_iter().find(|a| a.id == id)
}

pub fn save(config_dir: &Path, agent: Agent) -> Result<Vec<Agent>, String> {
    if !valid_id(&agent.id) {
        return Err("invalid agent id".into());
    }
    if agent.name.trim().is_empty() {
        return Err("An agent needs a name.".into());
    }
    let mut agents = load(config_dir);
    match agents.iter_mut().find(|a| a.id == agent.id) {
        Some(existing) => {
            crate::chat::versions::keep(config_dir, "agents", &agent.id, &*existing, &agent)?;
            *existing = agent;
        }
        None => agents.push(agent),
    }
    AGENTS_FILE.save(config_dir, &agents)?;
    Ok(agents)
}

pub fn delete(config_dir: &Path, id: &str) -> Result<Vec<Agent>, String> {
    let mut agents = load(config_dir);
    agents.retain(|a| a.id != id);
    AGENTS_FILE.save(config_dir, &agents)?;
    Ok(agents)
}

/// Whether `agent` may use the tool `name` (`connector__tool` or a built-in name).
pub fn allows(agent: &Agent, name: &str) -> bool {
    if agent.tools.is_empty() {
        return true;
    }
    // A group works through its members.
    if !agent.members.is_empty() && name == crate::chat::builtins::DELEGATE {
        return true;
    }
    // A manager reaches its team: it writes to its reports and hands them work.
    if agent.can_manage_agents && (name == MESSAGE_AGENT || name == crate::chat::builtins::DELEGATE)
    {
        return true;
    }
    // Libraries given to the agent come with the tools that read them.
    if !agent.libraries.is_empty() && matches!(name, "search_library" | "read_document") {
        return true;
    }
    let connector = name.split_once("__").map_or(name, |(c, _)| c);
    agent.tools.iter().any(|t| t == name || t == connector)
}

/// The tool an agent uses to keep its notebook.
pub const NOTEBOOK_TOOL: &str = "update_notebook";
/// Characters a notebook may hold; it goes into every prompt of the agent.
pub const NOTEBOOK_CHARS: usize = 12_000;

fn notebook_path(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("invalid agent id".into());
    }
    Ok(chat_dir(config_dir)
        .join("agents")
        .join(id)
        .join("notebook.md"))
}

/// The agent's notebook: what it keeps across conversations and runs, in Markdown.
pub fn notebook(config_dir: &Path, id: &str) -> String {
    notebook_path(config_dir, id)
        .ok()
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn save_notebook(config_dir: &Path, id: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.chars().count() > NOTEBOOK_CHARS {
        return Err(format!(
            "The notebook holds at most {NOTEBOOK_CHARS} characters; condense it."
        ));
    }
    let path = notebook_path(config_dir, id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // The notebook as it was stays restorable.
    crate::chat::versions::keep(
        config_dir,
        "notebooks",
        id,
        &notebook(config_dir, id),
        &text.to_string(),
    )?;
    crate::chat::at_rest::write(path, text).map_err(|e| e.to_string())
}

pub fn notebook_tool() -> cortex_kernel::tool::ChatTool {
    cortex_kernel::tool::ChatTool {
        name: NOTEBOOK_TOOL.into(),
        description: format!(
            "Rewrite your notebook: the working memory you keep across all your conversations and \
             scheduled runs. Keep it current and short (at most {NOTEBOOK_CHARS} characters): what \
             you learned about the work, decisions, preferences, open threads, and what to do \
             differently next time. Send the whole notebook, not a diff."
        ),
        parameters: serde_json::json!({ "type": "object", "properties": { "content": { "type": "string", "description": "The full notebook, in Markdown." } }, "required": ["content"] }),
        connector_id: None,
        tool_name: NOTEBOOK_TOOL.into(),
        read_only: true,
    }
}

/// The part of the agent's prompt about its notebook.
pub fn notebook_prompt(text: &str) -> String {
    let body = if text.trim().is_empty() {
        "(empty: start it when you learn something worth keeping)"
    } else {
        text.trim()
    };
    format!(
        "\n\n## Your notebook\nYou keep this across every conversation and run; the user can read and \
         edit it. Update it with `{NOTEBOOK_TOOL}` when you learn something that will matter next \
         time, before you finish.\n<notebook>\n{body}\n</notebook>"
    )
}

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
const SHARED_FILE_CHARS: usize = 60_000;

/// What the receiving agent gets: who wrote, the message, and the shared files' text.
pub fn message_text(from: &str, message: &str, files: &[(String, String)]) -> String {
    let mut out = format!("Message from {from}:\n\n{}", message.trim());
    for (name, text) in files {
        let text: String = text.chars().take(SHARED_FILE_CHARS).collect();
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
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for p in paths.iter().take(5) {
        let path = Path::new(p);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workdir
                .ok_or("Relative paths need the chat's folder; give a full path.")?
                .join(path)
        };
        let bytes = std::fs::read(&full).map_err(|e| format!("could not read {p}: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{p} is not a text file."))?;
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
) -> Result<crate::chat::libraries::Library, String> {
    let mut agent = find(config_dir, id).ok_or("No such agent.")?;
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

pub fn set_routing(config_dir: &Path, enabled: bool) -> Result<(), String> {
    crate::chat::store::write_json(
        &chat_dir(config_dir).join("agent-routing.json"),
        &serde_json::json!({ "enabled": enabled }),
    )
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
        message.chars().take(2_000).collect::<String>(),
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

/// What an agent's conversations cost this calendar month.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub runs: usize,
    pub tokens: u64,
    pub cost: f64,
}

pub fn spend_this_month(config_dir: &Path) -> std::collections::HashMap<String, Spend> {
    use chrono::Datelike;
    let now = Utc::now();
    let mut out: std::collections::HashMap<String, Spend> = Default::default();
    for c in crate::chat::store::list_conversations(config_dir) {
        let Some(agent) = c.agent_id.clone() else {
            continue;
        };
        if c.updated_at.year() != now.year() || c.updated_at.month() != now.month() {
            continue;
        }
        let entry = out.entry(agent).or_default();
        entry.runs += 1;
        entry.tokens += c.tokens;
        entry.cost += c.cost;
    }
    out
}

/// Refuses a turn when the agent's monthly budget is used up.
pub fn check_budget(config_dir: &Path, agent: &Agent) -> Result<(), String> {
    let Some(budget) = agent.monthly_budget.filter(|b| *b > 0.0) else {
        return Ok(());
    };
    // The trace database knows each run's cost; before any run is traced this month, the
    // conversations are added up as before.
    let month_start = {
        use chrono::{Datelike, TimeZone};
        let now = Utc::now();
        Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .unwrap_or(now)
    };
    let spent =
        crate::chat::traces::agent_cost(config_dir, &agent.id, month_start).unwrap_or_else(|| {
            spend_this_month(config_dir)
                .get(&agent.id)
                .map_or(0.0, |s| s.cost)
        });
    if spent >= budget {
        return Err(format!(
            "{} has used its monthly budget (${spent:.2} of ${budget:.2}). Raise the budget on its page to continue.",
            agent.name
        ));
    }
    Ok(())
}

/// The instruction that turns a one-line description into an agent.
pub fn draft_prompt(description: &str, tools: &[&str], skills: &[String]) -> String {
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
pub fn refine_prompt(current: &Agent, request: &str, tools: &[&str], skills: &[String]) -> String {
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
pub fn parse_draft(text: &str, tools: &[&str], skills: &[String]) -> Result<Agent, String> {
    let start = text.find('{').ok_or("The model did not return an agent.")?;
    let end = text
        .rfind('}')
        .ok_or("The model did not return an agent.")?;
    let value: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|_| "The model's draft was not valid JSON; try again.".to_string())?;
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
        return Err("The draft had no name; try again.".into());
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

/// Whether the agent's policy says to ask before every use of `tool`.
pub fn asks_first(agent: &Agent, tool: &str) -> bool {
    let connector = tool.split_once("__").map_or(tool, |(c, _)| c);
    agent.ask_tools.iter().any(|t| t == tool || t == connector)
}

/// The approval mode for one call: an agent's "ask" tools ask even when the mode would not.
pub fn mode_for(
    agent: Option<&Agent>,
    tool: &str,
    mode: cortex_kernel::approval::ApprovalMode,
) -> cortex_kernel::approval::ApprovalMode {
    use cortex_kernel::approval::ApprovalMode;
    let Some(agent) = agent else { return mode };
    let ask = asks_first(agent, tool);
    match mode {
        ApprovalMode::Auto | ApprovalMode::AcceptEdits if ask => ApprovalMode::Ask,
        // Nobody can answer during a scheduled run: an "ask" tool is refused.
        ApprovalMode::Autonomous if ask => ApprovalMode::Unattended,
        other => other,
    }
}

/// Ready-made agents a user can add with one click.
pub fn templates() -> Vec<Agent> {
    let make = |id: &str,
                name: &str,
                avatar: &str,
                description: &str,
                instructions: &str,
                tools: &[&str]| Agent {
        id: id.into(),
        name: name.into(),
        avatar: avatar.into(),
        description: description.into(),
        instructions: instructions.into(),
        model: None,
        tools: tools.iter().map(|s| s.to_string()).collect(),
        skills: Vec::new(),
        ask_tools: Vec::new(),
        think: false,
        fallback_model: None,
        monthly_budget: None,
        starters: Vec::new(),
        libraries: Vec::new(),
        secrets: Vec::new(),
        reminder: String::new(),
        members: Vec::new(),
        group_mode: String::new(),
        code_network: false,
        title: String::new(),
        can_manage_agents: false,
        reports_to: None,
        created_at: Utc::now(),
    };
    vec![
        make(
            "researcher",
            "Researcher",
            "🔎",
            "Finds, verifies and summarizes information from the web with sources.",
            "You are a meticulous research analyst. Plan before searching, prefer primary and official sources, cross-check key figures, delegate broad questions to sub-agents, and always cite. Flag uncertainty plainly.",
            &["web_search", "browser", "delegate", "update_plan", "canvas", "use_skill"],
        ),
        make(
            "news-briefer",
            "News Briefer",
            "📰",
            "Writes a concise briefing of what changed in a topic since last time.",
            "You write short, skimmable briefings: the 5 most important developments, one or two sentences each, with the date and a source link. Lead with what changed and why it matters. No filler.",
            &["web_search", "browser", "delegate"],
        ),
        make(
            "file-organizer",
            "File Assistant",
            "🗂️",
            "Searches, reads and summarizes documents in the folders you share.",
            "You help the user with the documents in their shared folders: find files, read them, summarize and compare them, and propose clear next steps. Never change a file unless asked.",
            &["filesystem", "update_plan", "canvas"],
        ),
        make(
            "writer",
            "Writer",
            "✍️",
            "Drafts and edits documents in a canvas, matching your tone.",
            "You are a precise editor and writer. Ask for audience and goal only when they cannot be inferred. Write in a canvas, keep the chat reply to a line, and offer one alternative when useful.",
            &["canvas", "web_search", "use_skill"],
        ),
        // Specialists for the apps and data on this computer (Osaurus's built-in app agents).
        make(
            "calendar-assistant",
            "Calendar Assistant",
            "📅",
            "Answers what is on your calendar, finds free time and who people are, and sets reminders.",
            "You manage the user's time. Read the calendar with calendar_events and people with find_contact before answering; give times in the user's time zone, and say plainly when something is unknown. To remind the user later, use schedule_task with a clear title. Never invent events.",
            &["calendar_events", "find_contact", "schedule_task", "notify"],
        ),
        make(
            "data-analyst",
            "Data Analyst",
            "📊",
            "Reads spreadsheets and CSV files, runs the numbers in Python and charts them.",
            "You are a careful data analyst. Load the data with run_code (pandas), check its shape and gaps first, then answer with the exact figures you computed and how. Make charts with matplotlib when they help, and put tables in a sheet canvas when the user will want to keep them. State assumptions; never guess numbers.",
            &["run_code", "canvas", "filesystem", "search_library", "read_document"],
        ),
        make(
            "code-reviewer",
            "Code Reviewer",
            "🔍",
            "Reviews a project's changes for bugs, security problems and clarity, without editing.",
            "You review code; you do not change it. Read the files and the git diff with read-only commands, then report findings ordered by severity, each with the file and line, what goes wrong in a concrete case, and a suggested fix. Say clearly when you found nothing serious.",
            &["filesystem", "bash", "update_plan"],
        ),
        make(
            "translator",
            "Translator",
            "🌍",
            "Translates text and documents naturally, keeping meaning, tone and formatting.",
            "You translate. Keep the meaning, register and formatting (Markdown, lists, code untouched), use the target language's natural phrasing rather than word-for-word, and keep names and terms consistent. When the target language is not given, ask once. Put long documents in a canvas.",
            &["canvas", "read_document", "search_library"],
        ),
        make(
            "meeting-prep",
            "Meeting Prep",
            "🗂️",
            "Prepares a one-page brief for your next meetings from your calendar, notes and the web.",
            "Prepare the user for meetings. Read upcoming events with calendar_events, look up attendees with find_contact and earlier chats with search_chats, and research companies with web_search. Write a short brief per meeting: who, why, open questions, and three things to say. Cite sources.",
            &["calendar_events", "find_contact", "search_chats", "read_chat", "knowledge_search", "web_search", "canvas"],
        ),
    ]
}

#[cfg(test)]
mod group_tests {
    use super::*;

    #[test]
    fn messages_carry_shared_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "Budget: 40k").unwrap();
        let files = read_shared(&["notes.md".into()], Some(dir.path())).unwrap();
        assert_eq!(files, [("notes.md".to_string(), "Budget: 40k".to_string())]);
        assert!(read_shared(&["notes.md".into()], None).is_err());
        assert!(read_shared(&["missing.md".into()], Some(dir.path())).is_err());
        let text = message_text("Researcher", "Check the budget.", &files);
        assert!(
            text.starts_with("Message from Researcher:")
                && text.contains("<attachment name=\"notes.md\">\nBudget: 40k")
                && text.ends_with("passed back.")
        );
    }

    #[test]
    fn agents_get_their_own_files_library() {
        let dir = tempfile::tempdir().unwrap();
        let agent = Agent {
            id: "a1".into(),
            name: "Researcher".into(),
            created_at: Utc::now(),
            ..Default::default()
        };
        save(dir.path(), agent).unwrap();
        let first = files_library(dir.path(), "a1").unwrap();
        assert_eq!(first.name, "Researcher files");
        assert_eq!(
            files_library(dir.path(), "a1").unwrap().id,
            first.id,
            "made once"
        );
        assert_eq!(find(dir.path(), "a1").unwrap().libraries, [first.id]);
        assert!(files_library(dir.path(), "nope").is_err());
    }

    #[test]
    fn new_chats_route_to_the_named_agent() {
        let make = |id: &str, name: &str| Agent {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let all = vec![
            make("a", "Researcher"),
            make("b", "Code Reviewer"),
            make("c", "Code"),
        ];
        assert_eq!(
            parse_route(&all, "Code Reviewer").map(|a| a.id.as_str()),
            Some("b")
        );
        assert_eq!(
            parse_route(&all, "**researcher**\n").map(|a| a.id.as_str()),
            Some("a")
        );
        assert_eq!(
            parse_route(&all, "The best fit is Code Reviewer.").map(|a| a.id.as_str()),
            Some("b"),
            "the longest name mentioned wins"
        );
        assert!(parse_route(&all, "none").is_none());
        assert!(parse_route(&all, "Translator").is_none());
        assert!(route_prompt(&all, "Review my PR").contains("- Code Reviewer: no description"));
        let dir = tempfile::tempdir().unwrap();
        assert!(!routing_enabled(dir.path()));
        set_routing(dir.path(), true).unwrap();
        assert!(routing_enabled(dir.path()));
    }

    #[test]
    fn groups_host_their_members() {
        let make = |id: &str, name: &str| Agent {
            id: id.into(),
            name: name.into(),
            description: format!("{name} work"),
            ..Default::default()
        };
        let all = vec![
            make("a", "Researcher"),
            make("b", "Critic"),
            make("g", "Board"),
        ];
        let mut group = all[2].clone();
        assert!(group_prompt(&all, &group).is_none(), "no members, no group");
        group.members = vec!["b".into(), "a".into(), "g".into(), "missing".into()];
        group.group_mode = "debate".into();
        group.tools = vec!["web_search".into()];
        let p = group_prompt(&all, &group).unwrap();
        assert!(
            p.contains("mode: \"debate\"")
                && p.find("Critic").unwrap() < p.find("Researcher").unwrap(),
            "{p}"
        );
        assert!(!p.contains("**Board**"), "a group is not its own member");
        assert!(
            allows(&group, crate::chat::builtins::DELEGATE),
            "a group may always delegate"
        );
        assert!(!allows(
            &Agent {
                tools: vec!["web_search".into()],
                ..Default::default()
            },
            crate::chat::builtins::DELEGATE
        ));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn agents_travel_as_bundles() {
        let from = tempfile::tempdir().unwrap();
        crate::chat::skills::save(
            from.path(),
            "triage-inbox",
            "Use when sorting email.",
            "1. Read.\n2. Sort.",
        )
        .unwrap();
        let agent = Agent {
            id: "a1".into(),
            name: "Inbox helper".into(),
            avatar: "📬".into(),
            description: String::new(),
            instructions: "Sort mail.".into(),
            model: None,
            tools: vec![],
            skills: vec!["triage-inbox".into(), "data-analysis".into()],
            ask_tools: vec![],
            think: false,
            fallback_model: None,
            monthly_budget: Some(5.0),
            starters: Vec::new(),
            libraries: Vec::new(),
            secrets: Vec::new(),
            reminder: String::new(),
            members: Vec::new(),
            group_mode: String::new(),
            code_network: false,
            title: String::new(),
            can_manage_agents: false,
            reports_to: None,
            created_at: Utc::now(),
        };
        save(from.path(), agent).unwrap();
        save_notebook(from.path(), "a1", "Boss emails first.").unwrap();
        let b = bundle(from.path(), "a1").unwrap();
        assert_eq!(
            b.skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["triage-inbox"],
            "built-in skills stay behind"
        );
        let json = serde_json::to_string(&b).unwrap();

        let to = tempfile::tempdir().unwrap();
        let installed = install(to.path(), serde_json::from_str(&json).unwrap()).unwrap();
        assert_ne!(installed.id, "a1");
        assert_eq!(notebook(to.path(), &installed.id), "Boss emails first.");
        assert!(crate::chat::skills::find(to.path(), "triage-inbox").is_some());
        let again = install(to.path(), serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(again.name, "Inbox helper (imported)");
        let mut wrong: Bundle = serde_json::from_str(&json).unwrap();
        wrong.format = "other".into();
        assert!(install(to.path(), wrong).is_err());
    }

    use super::*;

    #[test]
    fn shared_agents_carry_their_tool_servers_without_secrets() {
        let mut agent = templates().remove(0);
        agent.tools = vec!["web_search".into(), "mcp-github".into()];
        agent.ask_tools = vec!["mcp-linear__create_issue".into()];
        let servers = vec![
            serde_json::json!({ "id": "github", "display_name": "GitHub", "transport": "stdio", "command": "npx", "args": ["gh-mcp"], "env": { "GITHUB_TOKEN": "ghp_secret" }, "secret_env": [] }),
            serde_json::json!({ "id": "linear", "display_name": "Linear", "transport": "http", "url": "https://mcp.linear.app/sse", "auth": { "type": "oauth" } }),
            serde_json::json!({ "id": "notion", "display_name": "Notion", "transport": "http", "url": "https://mcp.notion.com", "auth": { "type": "none" } }),
        ];
        let shared = shareable_mcp_servers(&agent, servers);
        assert_eq!(
            shared
                .iter()
                .map(|s| s["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["github", "linear"],
            "only the servers it uses"
        );
        assert_eq!(
            shared[0]["env"]["GITHUB_TOKEN"], "",
            "values typed into env are not shared"
        );
    }

    #[test]
    fn agents_save_and_restrict_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = templates().remove(0);
        agent.id = "a1".into();
        save(dir.path(), agent.clone()).unwrap();
        assert_eq!(find(dir.path(), "a1").unwrap().name, "Researcher");
        assert!(allows(&agent, "web_search"));
        assert!(allows(&agent, "browser__navigate"));
        assert!(!allows(&agent, "bash__run"));
        assert!(!allows(&agent, "search_library"));
        agent.libraries = vec!["lib1".into()];
        assert!(
            allows(&agent, "search_library") && allows(&agent, "read_document"),
            "its libraries come with the tools that read them"
        );
        agent.tools.clear();
        assert!(allows(&agent, "bash__run"));
        assert!(save(
            dir.path(),
            Agent {
                name: " ".into(),
                ..agent.clone()
            }
        )
        .is_err());
        assert!(delete(dir.path(), "a1").unwrap().is_empty());
    }

    #[test]
    fn notebooks_drafts_and_teams() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(notebook(dir.path(), "a1"), "");
        save_notebook(dir.path(), "a1", "  ## Learned\n- Prefers tables  ").unwrap();
        assert_eq!(notebook(dir.path(), "a1"), "## Learned\n- Prefers tables");
        assert!(notebook_prompt("x").contains(NOTEBOOK_TOOL));
        assert!(save_notebook(dir.path(), "a1", &"x".repeat(NOTEBOOK_CHARS + 1)).is_err());
        assert!(save_notebook(dir.path(), "../evil", "x").is_err());

        let tools = ["web_search", "canvas"];
        let skills = vec!["deep-research".to_string()];
        let draft = parse_draft(
            "Sure:\n{\"name\": \"Market Watcher\", \"avatar\": \"📈\", \"description\": \"Tracks competitors\", \"instructions\": \"You watch.\", \"tools\": [\"web_search\", \"bash\"], \"skills\": [\"deep-research\", \"nope\"], \"think\": true}",
            &tools,
            &skills,
        )
        .unwrap();
        assert_eq!(
            (
                draft.name.as_str(),
                draft.tools.clone(),
                draft.skills.clone(),
                draft.think
            ),
            (
                "Market Watcher",
                vec!["web_search".to_string()],
                skills.clone(),
                true
            )
        );
        assert!(parse_draft("no json here", &tools, &skills).is_err());

        let team = templates();
        assert!(by_name(&team, "researcher").is_some());
        let prompt = team_prompt(&team, Some("researcher")).unwrap();
        assert!(prompt.contains("Writer") && !prompt.contains("**Researcher**"));
        assert!(team_prompt(&team[..1], Some("researcher")).is_none());
    }

    #[test]
    fn ask_tools_raise_the_approval_mode() {
        use cortex_kernel::approval::ApprovalMode;
        let mut agent = templates().remove(0);
        agent.ask_tools = vec!["bash".into()];
        assert_eq!(
            mode_for(Some(&agent), "bash__run", ApprovalMode::Auto),
            ApprovalMode::Ask
        );
        assert_eq!(
            mode_for(Some(&agent), "bash__run", ApprovalMode::Autonomous),
            ApprovalMode::Unattended
        );
        assert_eq!(
            mode_for(Some(&agent), "web_search", ApprovalMode::Auto),
            ApprovalMode::Auto
        );
        assert_eq!(
            mode_for(None, "bash__run", ApprovalMode::Auto),
            ApprovalMode::Auto
        );
    }

    #[test]
    fn budgets_stop_an_agent() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = templates().remove(0);
        let mut c = crate::chat::store::Conversation::new(
            "11111111-1111-4111-8111-111111111111".into(),
            None,
            None,
        );
        c.agent_id = Some(agent.id.clone());
        c.usage.cost = 1.5;
        c.usage.input_tokens = 1000;
        crate::chat::store::save_conversation(dir.path(), &c).unwrap();
        let spend = spend_this_month(dir.path());
        assert_eq!(
            spend[&agent.id],
            Spend {
                runs: 1,
                tokens: 1000,
                cost: 1.5
            }
        );
        assert!(check_budget(dir.path(), &agent).is_ok());
        agent.monthly_budget = Some(1.0);
        assert!(check_budget(dir.path(), &agent)
            .unwrap_err()
            .contains("monthly budget"));
        agent.monthly_budget = Some(2.0);
        assert!(check_budget(dir.path(), &agent).is_ok());
    }

    #[test]
    fn templates_are_valid() {
        for t in templates() {
            assert!(valid_id(&t.id) && !t.name.is_empty() && !t.instructions.is_empty());
        }
    }
}
