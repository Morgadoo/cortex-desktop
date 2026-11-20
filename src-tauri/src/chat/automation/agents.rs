//! Agents: named assistants with their own persona, model, tools and standing tasks.
//!
//! The workforce: each agent is a reusable configuration a conversation or a scheduled task runs
//! under. Its instructions replace the generic persona, its model overrides the composer's, and
//! its tool list narrows what it may use. Saved in `<config>/chat/agents.json`.

use crate::chat::error::ChatError;
use crate::support::settings_file::SettingsFile;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::chat::store::{chat_dir, valid_id};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Its job title ("3D Artist", "QA Lead"), shown next to its name: the role that sets its
    /// responsibilities in the team.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<String>", optional))]
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
    #[cfg_attr(feature = "typescript", ts(as = "Option<Vec<String>>", optional))]
    pub members: Vec<String>,
    /// How the members work: `parallel`, `sequential` or `debate`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<String>", optional))]
    pub group_mode: String,
    /// Its code may reach the internet (to install packages into its own environment).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<bool>", optional))]
    pub code_network: bool,
    /// It runs an organization (a manager or "CEO" agent): it may create, change and remove the
    /// agents under it with `manage_agents`. Off unless the user turns it on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<bool>", optional))]
    pub can_manage_agents: bool,
    /// The agent this one reports to, by id: the manager that created it or took it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub reports_to: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Agent {
    /// Its name with its title, as the team sees it: `João [3D Artist]`.
    pub(crate) fn label(&self) -> String {
        if self.title.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{} [{}]", self.name, self.title.trim())
        }
    }

    /// What its title means for how it works, for its own prompt; empty without a title.
    pub(in crate::chat) fn role_prompt(&self) -> String {
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

pub(crate) fn load(config_dir: &Path) -> Vec<Agent> {
    AGENTS_FILE.load(config_dir)
}

pub fn find(config_dir: &Path, id: &str) -> Option<Agent> {
    load(config_dir).into_iter().find(|a| a.id == id)
}

pub(crate) fn save(config_dir: &Path, agent: Agent) -> Result<Vec<Agent>, ChatError> {
    if !valid_id(&agent.id) {
        return Err(ChatError::Invalid("invalid agent id".into()));
    }
    if agent.name.trim().is_empty() {
        return Err(ChatError::Invalid("An agent needs a name.".into()));
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

pub(crate) fn delete(config_dir: &Path, id: &str) -> Result<Vec<Agent>, ChatError> {
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

/// Whether the agent's policy says to ask before every use of `tool`.
pub(in crate::chat) fn asks_first(agent: &Agent, tool: &str) -> bool {
    let connector = tool.split_once("__").map_or(tool, |(c, _)| c);
    agent.ask_tools.iter().any(|t| t == tool || t == connector)
}

/// The approval mode for one call: an agent's "ask" tools ask even when the mode would not.
pub(in crate::chat) fn mode_for(
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

mod budget;
mod bundles;
mod drafting;
mod notebook;
mod team;
mod templates;
pub use budget::*;
pub use bundles::*;
pub(in crate::chat) use drafting::*;
pub use notebook::*;
pub use team::*;
pub(in crate::chat) use templates::*;

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
            .to_string()
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
