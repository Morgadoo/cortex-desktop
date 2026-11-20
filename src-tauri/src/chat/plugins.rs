//! Claude plugins and plugin marketplaces brought in as Cortex's own parts (Osaurus imports Claude
//! plugins too). A plugin is a folder:
//!
//! - `skills/<name>/SKILL.md` become skills, as they are;
//! - `commands/<name>.md` (slash commands) become skills used with `/name`;
//! - `agents/<name>.md` (subagents) become agents, their body the instructions;
//! - `.mcp.json` servers become tool servers under Connectors.
//!
//! A marketplace (`.claude-plugin/marketplace.json`) brings every plugin it holds in its own
//! folder. Hooks are not run. The source is a folder on disk or a GitHub repository, read from
//! its ZIP download; files are read into memory first, so parsing is the same for both.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// Files by path relative to the source's root, `/`-separated.
pub type Files = BTreeMap<String, Vec<u8>>;

const MAX_FILES: usize = 3_000;
const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_TOTAL: usize = 60 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub body: String,
    /// Supporting files from the skill's folder (path inside it, bytes).
    pub files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentDraft {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerDraft {
    pub name: String,
    /// `{"command", "args", "env"}` or `{"type": "http", "url", "headers"}`, as `.mcp.json` has it.
    pub spec: Value,
}

#[derive(Debug, Clone, Default)]
pub struct PluginDraft {
    pub name: String,
    pub description: String,
    /// The plugin's folder inside the source (empty for the root).
    pub root: String,
    pub skills: Vec<SkillDraft>,
    pub agents: Vec<AgentDraft>,
    pub servers: Vec<ServerDraft>,
    pub has_hooks: bool,
    /// `hooks/hooks.json`, or the manifest's `hooks` object.
    pub hooks: Option<Value>,
}

/// What an import brought in, for the message the user sees.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub plugins: Vec<String>,
    pub skills: Vec<String>,
    pub agents: Vec<String>,
    pub servers: Vec<String>,
    /// Hooks added, turned off until the user reviews them.
    pub hooks: usize,
    pub skipped: Vec<String>,
}

fn add(files: &mut Files, total: &mut usize, path: String, bytes: Vec<u8>) -> Result<(), String> {
    if files.len() >= MAX_FILES || *total + bytes.len() > MAX_TOTAL {
        return Err("That source is too large to import.".into());
    }
    *total += bytes.len();
    files.insert(path, bytes);
    Ok(())
}

fn skipped_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | ".venv" | "__pycache__" | "dist"
    )
}

/// The files of a folder on disk.
pub fn from_dir(root: &Path) -> Result<Files, String> {
    let mut files = Files::new();
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !skipped_dir(&name) {
                    stack.push(path);
                }
            } else if kind.is_file() && entry.metadata().is_ok_and(|m| m.len() as usize <= MAX_FILE)
            {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                add(
                    &mut files,
                    &mut total,
                    rel,
                    crate::chat::at_rest::read(&path).map_err(|e| e.to_string())?,
                )?;
            }
        }
    }
    Ok(files)
}

/// The files of a ZIP archive; a single folder holding everything (as GitHub's downloads have)
/// is taken off the paths.
pub fn from_zip(bytes: &[u8]) -> Result<Files, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a ZIP archive: {e}"))?;
    let mut files = Files::new();
    let mut total = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file() || entry.size() as usize > MAX_FILE {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let rel = path.to_string_lossy().replace('\\', "/");
        if rel.split('/').any(skipped_dir) {
            continue;
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data).map_err(|e| e.to_string())?;
        add(&mut files, &mut total, rel, data)?;
    }
    let first = files
        .keys()
        .next()
        .and_then(|k| k.split_once('/'))
        .map(|(top, _)| format!("{top}/"));
    if let Some(top) = first.filter(|top| files.keys().all(|k| k.starts_with(top.as_str()))) {
        files = files
            .into_iter()
            .map(|(k, v)| (k[top.len()..].to_string(), v))
            .collect();
    }
    Ok(files)
}

/// The ZIP downloads to try for a GitHub link, and the folder inside the repository it points
/// at: `github.com/owner/repo`, or `…/tree/<ref>/<folder>`.
pub fn github_sources(link: &str) -> Result<(Vec<String>, String), String> {
    let rest = link.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .unwrap_or(rest);
    let path = rest
        .strip_prefix("github.com/")
        .ok_or("Give a folder on this computer or a github.com link.")?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [owner, repo] => Ok((
            ["main", "master"]
                .iter()
                .map(|r| format!("https://codeload.github.com/{owner}/{repo}/zip/refs/heads/{r}"))
                .collect(),
            String::new(),
        )),
        [owner, repo, "tree", reference, folder @ ..] => Ok((
            vec![format!(
                "https://codeload.github.com/{owner}/{repo}/zip/{reference}"
            )],
            folder.join("/"),
        )),
        _ => Err("Give a link to a repository, like https://github.com/owner/repo.".into()),
    }
}

/// One plugin a marketplace lists, and the GitHub link it is installed from (none when its
/// source is somewhere this app cannot fetch, such as a package registry).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketEntry {
    pub name: String,
    pub description: String,
    pub category: String,
    pub link: Option<String>,
}

/// `owner/repo` of a GitHub repository URL or `owner/repo` string.
fn github_repo(spec: &str) -> Option<String> {
    let rest = spec.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .unwrap_or(rest);
    let rest = rest
        .strip_prefix("github.com/")
        .or_else(|| rest.strip_prefix("git@github.com:"))
        .unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [owner, repo] if !owner.is_empty() && !repo.is_empty() && !owner.contains('.') => {
            Some(format!("{owner}/{repo}"))
        }
        _ => None,
    }
}

/// A browsable link for a repository, a ref and a folder in it.
fn tree_link(repo: &str, reference: Option<&str>, path: Option<&str>) -> String {
    let path = path
        .map(|p| p.trim_start_matches("./").trim_matches('/'))
        .filter(|p| !p.is_empty());
    match (reference, path) {
        (_, None) if reference.is_none() => format!("https://github.com/{repo}"),
        (reference, path) => format!(
            "https://github.com/{repo}/tree/{}{}",
            reference.unwrap_or("main"),
            path.map(|p| format!("/{p}")).unwrap_or_default()
        ),
    }
}

/// The plugins a `marketplace.json` lists. `home` is the marketplace's own repository and ref,
/// which plugins kept inside it (`"source": "./plugins/x"`) come from.
pub fn market_entries(market: &Value, home: (&str, &str)) -> Vec<MarketEntry> {
    let base = market
        .pointer("/metadata/pluginRoot")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string();
    market["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p["name"].as_str()?.to_string();
            let source = &p["source"];
            let link = if let Some(local) = source.as_str() {
                let local = local.trim_start_matches("./").trim_matches('/');
                let path = if local.contains('/') || base.is_empty() {
                    local.to_string()
                } else {
                    format!("{base}/{local}")
                };
                Some(tree_link(home.0, Some(home.1), Some(&path)))
            } else {
                let repo = source["repo"]
                    .as_str()
                    .and_then(github_repo)
                    .or_else(|| source["url"].as_str().and_then(github_repo));
                repo.map(|repo| {
                    tree_link(
                        &repo,
                        source["ref"].as_str().or(if source["path"].is_string() {
                            Some("main")
                        } else {
                            None
                        }),
                        source["path"].as_str(),
                    )
                })
            };
            Some(MarketEntry {
                name,
                description: p["description"].as_str().unwrap_or_default().to_string(),
                category: p["category"].as_str().unwrap_or_default().to_string(),
                link,
            })
        })
        .collect()
}

/// Reads the marketplace a GitHub link points at, from its raw file; `None` when the
/// repository has no `.claude-plugin/marketplace.json` (it is then a plugin itself).
pub async fn read_market(link: &str) -> Result<Option<Vec<MarketEntry>>, String> {
    let rest = link.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .unwrap_or(rest);
    let path = rest
        .strip_prefix("github.com/")
        .ok_or("Give a github.com link.")?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (repo, refs, folder): (String, Vec<String>, String) = match parts.as_slice() {
        [owner, repo] => (
            format!("{owner}/{repo}"),
            vec!["main".into(), "master".into()],
            String::new(),
        ),
        [owner, repo, "tree", reference, folder @ ..] => (
            format!("{owner}/{repo}"),
            vec![(*reference).to_string()],
            folder.join("/"),
        ),
        _ => return Err("Give a link to a repository, like https://github.com/owner/repo.".into()),
    };
    let client = crate::support::http::client(std::time::Duration::from_secs(30))?;
    for reference in refs {
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        let url = format!("https://raw.githubusercontent.com/{repo}/{reference}/{prefix}.claude-plugin/marketplace.json");
        let Ok(response) = client.get(&url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let market: Value = response
            .json()
            .await
            .map_err(|e| format!("The marketplace file could not be read: {e}"))?;
        return Ok(Some(market_entries(&market, (&repo, &reference))));
    }
    Ok(None)
}

/// Only the files under `folder`, with the folder taken off their paths.
pub fn subfolder(files: Files, folder: &str) -> Files {
    if folder.is_empty() {
        return files;
    }
    let prefix = format!("{}/", folder.trim_matches('/'));
    files
        .into_iter()
        .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|rest| (rest.to_string(), v)))
        .collect()
}

/// Front matter `key: value` lines and the body of a Markdown file.
pub fn front_matter(text: &str) -> (BTreeMap<String, String>, String) {
    let text = text.trim_start_matches('\u{feff}');
    let mut map = BTreeMap::new();
    let Some(rest) = text.trim_start().strip_prefix("---") else {
        return (map, text.trim().to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (map, text.trim().to_string());
    };
    let (front, body) = rest.split_at(end);
    for line in front.lines() {
        if let Some((key, value)) = line.split_once(':') {
            map.insert(
                key.trim().to_lowercase(),
                value.trim().trim_matches(['"', '\'']).to_string(),
            );
        }
    }
    let body = body.trim_start_matches("\n---").trim().to_string();
    (map, body)
}

/// A name as skills and ids allow it: lowercase letters, digits and hyphens.
pub fn slug(name: &str) -> String {
    crate::support::names::slug(name, 60)
}

fn text(files: &Files, path: &str) -> Option<String> {
    files
        .get(path)
        .map(|b| String::from_utf8_lossy(b).into_owned())
}

fn join(root: &str, path: &str) -> String {
    if root.is_empty() {
        path.to_string()
    } else {
        format!("{root}/{path}")
    }
}

/// Where the plugins are: those a marketplace lists in its own folders, every folder with a
/// `.claude-plugin/plugin.json`, or the root when it holds plugin parts itself.
pub fn plugin_roots(files: &Files) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    if let Some(market) = text(files, ".claude-plugin/marketplace.json")
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let base = market
            .pointer("/metadata/pluginRoot")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_start_matches("./")
            .trim_matches('/')
            .to_string();
        for plugin in market["plugins"].as_array().into_iter().flatten() {
            if let Some(source) = plugin["source"].as_str() {
                let source = source.trim_start_matches("./").trim_matches('/');
                let root = if source.contains('/') || base.is_empty() {
                    source.to_string()
                } else {
                    join(&base, source)
                };
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
    }
    for path in files.keys() {
        if let Some(root) = path.strip_suffix(".claude-plugin/plugin.json") {
            let root = root.trim_end_matches('/').to_string();
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    if roots.is_empty()
        && files.keys().any(|k| {
            k.starts_with("skills/")
                || k.starts_with("commands/")
                || k.starts_with("agents/")
                || k == ".mcp.json"
        })
    {
        roots.push(String::new());
    }
    roots
}

/// A slash command's file as a skill: `$ARGUMENTS` becomes what the user writes after it.
fn command_skill(name: &str, text: &str) -> SkillDraft {
    let (front, body) = front_matter(text);
    let description = front
        .get("description")
        .cloned()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| format!("The /{name} command."));
    let body = inline_commands(&body)
        .replace("$ARGUMENTS", "(the text the user wrote after the command)")
        .replace("$1", "(the first thing the user wrote after the command)");
    SkillDraft {
        name: slug(name),
        description,
        body,
        files: Vec::new(),
    }
}

/// Claude Code runs `` !`command` `` in a command file and puts its output there; here the model
/// is asked to run it, with the tools it has.
fn inline_commands(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("!`") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('`') else { break };
        out.push_str(&rest[..start]);
        out.push_str(&format!("(run `{}` and use its output)", &after[..end]));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads one plugin.
pub fn read_plugin(files: &Files, root: &str) -> PluginDraft {
    let manifest = text(files, &join(root, ".claude-plugin/plugin.json"))
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);
    let folder_name = root
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("plugin");
    let name = manifest["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| folder_name.to_string());
    let mut plugin = PluginDraft {
        description: manifest["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        root: root.to_string(),
        ..Default::default()
    };
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{root}/")
    };
    for (path, bytes) in files.range(prefix.clone()..) {
        let Some(rel) = path.strip_prefix(&prefix) else {
            break;
        };
        let body = String::from_utf8_lossy(bytes);
        let parts: Vec<&str> = rel.split('/').collect();
        match parts.as_slice() {
            ["skills", skill, "SKILL.md"] => {
                let (front, text) = front_matter(&body);
                let name = front
                    .get("name")
                    .map(|n| slug(n))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| slug(skill));
                // The rest of the skill's folder comes with it.
                let folder = format!("{prefix}skills/{skill}/");
                let files = files
                    .range(folder.clone()..)
                    .take_while(|(k, _)| k.starts_with(&folder))
                    .filter(|(k, _)| k.as_str() != format!("{folder}SKILL.md"))
                    .map(|(k, v)| (k[folder.len()..].to_string(), v.clone()))
                    .collect();
                plugin.skills.push(SkillDraft {
                    name,
                    description: front.get("description").cloned().unwrap_or_default(),
                    body: text,
                    files,
                });
            }
            ["commands", .., file] if file.ends_with(".md") => plugin
                .skills
                .push(command_skill(file.trim_end_matches(".md"), &body)),
            ["agents", file] if file.ends_with(".md") => {
                let (front, text) = front_matter(&body);
                let name = front
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| file.trim_end_matches(".md").replace(['-', '_'], " "));
                plugin.agents.push(AgentDraft {
                    name,
                    description: front.get("description").cloned().unwrap_or_default(),
                    instructions: text,
                });
            }
            ["hooks", "hooks.json"] => {
                plugin.has_hooks = true;
                plugin.hooks = serde_json::from_str(&body).ok();
            }
            _ => {}
        }
    }
    // Servers come from `.mcp.json`, or from `mcpServers` in the manifest.
    let servers = text(files, &join(root, ".mcp.json"))
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| v.get("mcpServers").cloned().unwrap_or(v))
        .or_else(|| {
            manifest
                .get("mcpServers")
                .filter(|v| v.is_object())
                .cloned()
        });
    if let Some(Value::Object(map)) = servers {
        for (server, spec) in map {
            if spec.is_object() {
                plugin.servers.push(ServerDraft { name: server, spec });
            }
        }
    }
    if plugin.hooks.is_none() {
        if let Some(hooks) = manifest.get("hooks").filter(|v| v.is_object()) {
            plugin.has_hooks = true;
            plugin.hooks = Some(hooks.clone());
        }
    }
    plugin.name = name;
    plugin
}

/// A server from `.mcp.json` as a Connectors tool server. `${CLAUDE_PLUGIN_ROOT}` becomes the
/// folder the plugin's files were kept in.
pub fn server_config(
    plugin: &str,
    server: &ServerDraft,
    plugin_dir: &Path,
) -> Option<crate::tools::connectors::mcp::McpServerConfig> {
    use crate::tools::connectors::mcp::{HttpAuth, McpServerConfig, McpTransport};
    let root = plugin_dir.to_string_lossy();
    let fill = |s: &str| s.replace("${CLAUDE_PLUGIN_ROOT}", &root);
    let strings = |v: &Value| -> std::collections::HashMap<String, String> {
        v.as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), fill(s))))
            .collect()
    };
    let spec = &server.spec;
    let transport = if let Some(url) = spec["url"].as_str() {
        McpTransport::Http {
            url: fill(url),
            auth: HttpAuth::None,
            request_headers: strings(&spec["headers"]),
        }
    } else {
        McpTransport::Stdio {
            command: fill(spec["command"].as_str()?),
            args: spec["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(fill)
                .collect(),
            env: strings(&spec["env"]),
            secret_env: Vec::new(),
        }
    };
    Some(McpServerConfig {
        id: slug(&format!("{plugin}-{}", server.name)),
        display_name: server.name.clone(),
        description: Some(format!("From the {plugin} plugin.")),
        transport,
    })
}

/// Where a plugin's files are kept, for servers that run them.
pub fn plugin_dir(config_dir: &Path, name: &str) -> PathBuf {
    crate::chat::store::chat_dir(config_dir)
        .join("plugins")
        .join(slug(name))
}

/// Keeps the plugin's files on disk, replacing an earlier copy.
pub fn keep_files(files: &Files, root: &str, dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{root}/")
    };
    for (path, bytes) in files.iter().filter(|(k, _)| k.starts_with(&prefix)) {
        let rel = &path[prefix.len()..];
        if rel.split('/').any(|p| p == ".." || p.is_empty()) {
            continue;
        }
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::chat::at_rest::write(&target, bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if rel.ends_with(".sh") || rel.starts_with("bin/") || rel.starts_with("scripts/") {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(())
}

/// Saves a plugin's skills and agents. A skill whose name is taken by a different skill gets the
/// plugin's name in front; an agent of the same name is updated.
pub fn install(config_dir: &Path, plugin: &PluginDraft, out: &mut Imported) -> Result<(), String> {
    for skill in &plugin.skills {
        if !crate::chat::skills::valid_name(&skill.name) || skill.body.trim().is_empty() {
            out.skipped.push(format!("skill {}", skill.name));
            continue;
        }
        let mut name = skill.name.clone();
        if crate::chat::skills::find(config_dir, &name)
            .is_some_and(|s| s.body.trim() != skill.body.trim())
        {
            name = slug(&format!("{}-{}", plugin.name, skill.name));
        }
        crate::chat::skills::save(config_dir, &name, &skill.description, &skill.body)?;
        for (rel, bytes) in &skill.files {
            if let Err(e) = crate::chat::skills::add_file(config_dir, &name, rel, bytes) {
                out.skipped.push(format!("{rel} in skill {name} ({e})"));
            }
        }
        out.skills.push(name);
    }
    let existing = crate::chat::agents::load(config_dir);
    for draft in &plugin.agents {
        if draft.instructions.trim().is_empty() {
            out.skipped.push(format!("agent {}", draft.name));
            continue;
        }
        let id = existing
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&draft.name))
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), |a| a.id.clone());
        let mut agent: crate::chat::agents::Agent = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": draft.name,
            "description": draft.description,
            "instructions": draft.instructions,
            "createdAt": chrono::Utc::now(),
        }))
        .map_err(|e| e.to_string())?;
        if let Some(old) = existing.iter().find(|a| a.id == agent.id) {
            agent.created_at = old.created_at;
            agent.avatar.clone_from(&old.avatar);
        }
        crate::chat::agents::save(config_dir, agent)?;
        out.agents.push(draft.name.clone());
    }
    // Hooks run commands, so they arrive turned off; the user turns them on under Hooks.
    match &plugin.hooks {
        Some(config) => {
            let (hooks, unsupported) = crate::chat::hooks::from_claude(
                config,
                &plugin_dir(config_dir, &plugin.name),
                &plugin.name,
            );
            out.hooks += hooks.len();
            crate::chat::hooks::replace_from(config_dir, &plugin.name, hooks)?;
            out.skipped.extend(
                unsupported
                    .into_iter()
                    .map(|e| format!("{e} hooks (no such moment in Cortex)")),
            );
        }
        None if plugin.has_hooks => out
            .skipped
            .push(format!("{}'s hooks (unreadable hooks.json)", plugin.name)),
        None => {}
    }
    out.plugins.push(plugin.name.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &str)]) -> Files {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn a_plugin_maps_to_skills_agents_and_servers() {
        let f = files(&[
            (".claude-plugin/plugin.json", r#"{"name": "review-kit", "description": "Code review helpers"}"#),
            ("skills/pr-summary/SKILL.md", "---\nname: pr-summary\ndescription: Summarize a pull request\n---\nRead the diff and summarize it."),
            ("commands/review.md", "---\ndescription: Review the code\n---\nReview $ARGUMENTS for bugs."),
            ("commands/git/commit.md", "Write a commit message."),
            ("agents/security-reviewer.md", "---\nname: Security Reviewer\ndescription: Finds vulnerabilities\ntools: Read, Grep\n---\nYou review code for security issues."),
            (".mcp.json", r#"{"mcpServers": {"tracker": {"command": "${CLAUDE_PLUGIN_ROOT}/bin/tracker", "args": ["--port", "1"], "env": {"MODE": "x"}}, "docs": {"type": "http", "url": "https://example.com/mcp"}}}"#),
            ("hooks/hooks.json", r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh"}]}]}}"#),
            ("skills/pr-summary/template.md", "## Summary"),
            ("skills/pr-summary/scripts/diffstat.sh", "git diff --stat"),
            ("README.md", "readme"),
        ]);
        assert_eq!(plugin_roots(&f), [""]);
        let p = read_plugin(&f, "");
        assert_eq!(p.name, "review-kit");
        let names: Vec<&str> = p.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["commit", "review", "pr-summary"]);
        let review = &p.skills[1];
        assert_eq!(review.description, "Review the code");
        assert!(review
            .body
            .contains("(the text the user wrote after the command)"));
        assert_eq!(p.skills[0].description, "The /commit command.");
        assert_eq!(
            p.agents,
            [AgentDraft {
                name: "Security Reviewer".into(),
                description: "Finds vulnerabilities".into(),
                instructions: "You review code for security issues.".into()
            }]
        );
        assert!(p.has_hooks);
        let summary = p.skills.iter().find(|s| s.name == "pr-summary").unwrap();
        let paths: Vec<&str> = summary.files.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(paths, ["scripts/diffstat.sh", "template.md"]);
        let dir = tempfile::tempdir().unwrap();
        let mut out = Imported::default();
        install(dir.path(), &p, &mut out).unwrap();
        assert_eq!(out.hooks, 1);
        let hooks = crate::chat::hooks::load(dir.path());
        assert!(
            !hooks[0].enabled && hooks[0].command.ends_with("plugins/review-kit/guard.sh"),
            "{hooks:?}"
        );
        assert_eq!(
            crate::chat::skills::read_file(dir.path(), "pr-summary", "template.md").unwrap(),
            "## Summary"
        );
        assert_eq!(p.servers.len(), 2);
        let tracker = server_config(
            &p.name,
            p.servers.iter().find(|s| s.name == "tracker").unwrap(),
            Path::new("/data/plugins/review-kit"),
        )
        .unwrap();
        assert_eq!(tracker.id, "review-kit-tracker");
        match tracker.transport {
            crate::tools::connectors::mcp::McpTransport::Stdio {
                command, args, env, ..
            } => {
                assert_eq!(command, "/data/plugins/review-kit/bin/tracker");
                assert_eq!(args, ["--port", "1"]);
                assert_eq!(env.get("MODE").map(String::as_str), Some("x"));
            }
            _ => panic!("stdio expected"),
        }
        let docs = server_config(
            &p.name,
            p.servers.iter().find(|s| s.name == "docs").unwrap(),
            Path::new("/x"),
        )
        .unwrap();
        assert!(
            matches!(docs.transport, crate::tools::connectors::mcp::McpTransport::Http { ref url, .. } if url == "https://example.com/mcp")
        );
    }

    #[test]
    fn a_marketplace_lists_its_plugins() {
        let f = files(&[
            (
                ".claude-plugin/marketplace.json",
                r#"{"name": "m", "plugins": [{"name": "a", "source": "./plugins/a"}, {"name": "remote", "source": {"source": "github", "repo": "x/y"}}]}"#,
            ),
            ("plugins/a/commands/hello.md", "Say hello."),
            ("plugins/b/.claude-plugin/plugin.json", r#"{"name": "b"}"#),
            (
                "plugins/b/skills/s/SKILL.md",
                "---\nname: s\ndescription: d\n---\nbody",
            ),
        ]);
        assert_eq!(plugin_roots(&f), ["plugins/a", "plugins/b"]);
        assert_eq!(read_plugin(&f, "plugins/a").name, "a");
        assert_eq!(read_plugin(&f, "plugins/a").skills[0].name, "hello");
        assert_eq!(read_plugin(&f, "plugins/b").skills[0].body, "body");
    }

    #[test]
    fn links_zips_and_names() {
        let (urls, folder) = github_sources("https://github.com/acme/kit").unwrap();
        assert_eq!(
            urls[0],
            "https://codeload.github.com/acme/kit/zip/refs/heads/main"
        );
        assert!(folder.is_empty());
        let (urls, folder) = github_sources("github.com/acme/kit/tree/v2/plugins/review").unwrap();
        assert_eq!(urls, ["https://codeload.github.com/acme/kit/zip/v2"]);
        assert_eq!(folder, "plugins/review");
        assert!(github_sources("https://example.com/x").is_err());
        assert_eq!(slug("Security Reviewer!"), "security-reviewer");
        let sub = subfolder(
            files(&[("plugins/review/a.md", "x"), ("other.md", "y")]),
            "plugins/review",
        );
        assert_eq!(sub.keys().collect::<Vec<_>>(), ["a.md"]);

        // A GitHub download keeps everything in one top folder, which is taken off.
        let mut zip_bytes = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("kit-main/commands/hello.md", opts).unwrap();
            std::io::Write::write_all(&mut w, b"Say hello.").unwrap();
            w.start_file("kit-main/node_modules/x.js", opts).unwrap();
            std::io::Write::write_all(&mut w, b"skip").unwrap();
            w.finish().unwrap();
        }
        let f = from_zip(&zip_bytes).unwrap();
        assert_eq!(f.keys().collect::<Vec<_>>(), ["commands/hello.md"]);
    }

    #[test]
    fn marketplace_sources_resolve_to_links() {
        let market: Value = serde_json::from_str(r#"{
            "metadata": {"pluginRoot": "./plugins"},
            "plugins": [
                {"name": "agent-sdk-dev", "description": "Agent SDK kit", "category": "development",
                 "source": {"source": "git-subdir", "url": "https://github.com/anthropics/claude-plugins-official.git", "path": "plugins/agent-sdk-dev", "ref": "main"}},
                {"name": "local", "source": "./local"},
                {"name": "deep", "source": "./tools/deep"},
                {"name": "gh", "source": {"source": "github", "repo": "acme/review-plugin"}},
                {"name": "gh-ref", "source": {"source": "github", "repo": "acme/kit", "ref": "v2"}},
                {"name": "npm", "source": {"source": "npm", "package": "@acme/plugin"}},
                {"name": "gitlab", "source": {"source": "url", "url": "https://gitlab.com/acme/p.git"}}
            ]}"#).unwrap();
        let entries = market_entries(&market, ("acme/market", "main"));
        let links: Vec<(&str, Option<&str>)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.link.as_deref()))
            .collect();
        assert_eq!(links, [
            ("agent-sdk-dev", Some("https://github.com/anthropics/claude-plugins-official/tree/main/plugins/agent-sdk-dev")),
            ("local", Some("https://github.com/acme/market/tree/main/plugins/local")),
            ("deep", Some("https://github.com/acme/market/tree/main/tools/deep")),
            ("gh", Some("https://github.com/acme/review-plugin")),
            ("gh-ref", Some("https://github.com/acme/kit/tree/v2")),
            ("npm", None),
            ("gitlab", None),
        ]);
        assert_eq!(entries[0].category, "development");
        // Every link resolves to a download the importer knows.
        for link in entries.iter().filter_map(|e| e.link.as_deref()) {
            assert!(github_sources(link).is_ok(), "{link}");
        }
    }

    #[test]
    fn commands_that_run_shell_lines_ask_the_model_to_run_them() {
        let real = "---\nallowed-tools: Bash(git add:*), Bash(git commit:*)\ndescription: Create a git commit\n---\n\n## Context\n\n- Current git status: !`git status`\n- Current branch: !`git branch --show-current`\n\n## Your task\n\nCreate a single git commit. $ARGUMENTS";
        let skill = command_skill("commit", real);
        assert_eq!(skill.description, "Create a git commit");
        assert!(skill
            .body
            .contains("- Current git status: (run `git status` and use its output)"));
        assert!(skill
            .body
            .contains("(run `git branch --show-current` and use its output)"));
        assert!(!skill.body.contains("!`"));
        assert!(!skill.body.contains("allowed-tools"));
        assert_eq!(
            inline_commands("no commands, just `code`"),
            "no commands, just `code`"
        );
        assert_eq!(inline_commands("broken !`tail"), "broken !`tail");
    }
}
