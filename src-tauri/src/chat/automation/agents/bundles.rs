//! An agent as one file to share: its notebook, skills and MCP servers, exported and installed.

use super::*;
use crate::chat::error::ChatError;

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

pub fn bundle(config_dir: &Path, id: &str) -> Result<Bundle, ChatError> {
    let agent =
        find(config_dir, id).ok_or_else(|| ChatError::Invalid("There is no such agent.".into()))?;
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
pub fn install(config_dir: &Path, bundle: Bundle) -> Result<Agent, ChatError> {
    // `lumo-agent` files were exported before the rename to Cortex.
    if !matches!(bundle.format.as_str(), "cortex-agent" | "lumo-agent") {
        return Err(ChatError::Invalid(
            "This file is not an exported agent.".into(),
        ));
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
