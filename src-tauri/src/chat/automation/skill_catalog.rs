//! Browse skill catalogues (LobeHub's skills marketplace): public repositories that list their
//! skills in `.claude-plugin/marketplace.json`, starting with Anthropic's own (`anthropics/skills`).
//! The app reads the list and each skill's name and description, so you can search them and add
//! one as a skill of your own; other catalogues are added by `owner/repo`.
//!
//! Everything comes from raw.githubusercontent.com. Adding a skill saves its instructions, and its
//! scripts and other files where GitHub can list the skill's folder.

use crate::chat::error::ChatError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT: &str = "anthropics/skills";
const CACHE_HOURS: i64 = 24;
/// Files fetched with a skill, at most.
const MAX_FILES: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// `owner/repo`.
    pub catalog: String,
    pub branch: String,
    /// The skill's folder in the repository, like `skills/pdf`.
    pub path: String,
    pub name: String,
    pub description: String,
    /// The group it is listed in (the plugin's name).
    pub group: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Store {
    catalogs: Vec<String>,
    fetched_at: Option<chrono::DateTime<chrono::Utc>>,
    listings: Vec<Listing>,
}

fn path(config_dir: &Path) -> PathBuf {
    crate::chat::store::chat_dir(config_dir).join("skill-catalogs.json")
}

fn load(config_dir: &Path) -> Store {
    let mut s: Store = crate::chat::store::read_json(&path(config_dir)).unwrap_or_default();
    if s.catalogs.is_empty() {
        s.catalogs.push(DEFAULT.into());
    }
    s
}

fn save(config_dir: &Path, s: &Store) -> Result<(), ChatError> {
    crate::chat::store::write_json(&path(config_dir), s)
        .map_err(String::from)
        .map_err(ChatError::Invalid)
}

/// `owner/repo` from what the user typed: the pair itself or a GitHub link.
pub(in crate::chat::automation) fn repo_of(input: &str) -> Option<String> {
    let t = input.trim().trim_end_matches('/').trim_end_matches(".git");
    let t = t
        .strip_prefix("https://github.com/")
        .or_else(|| t.strip_prefix("github.com/"))
        .unwrap_or(t);
    let mut parts = t.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

fn raw_base() -> String {
    std::env::var("CORTEX_GITHUB_RAW")
        .unwrap_or_else(|_| "https://raw.githubusercontent.com".into())
}

fn client() -> Result<reqwest::Client, ChatError> {
    crate::support::http::client(Duration::from_secs(20)).map_err(ChatError::Invalid)
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, ChatError> {
    let r = client.get(url).send().await.map_err(ChatError::internal)?;
    if !r.status().is_success() {
        return Err(ChatError::Invalid(format!("{url} answered {}", r.status())));
    }
    r.text().await.map_err(ChatError::internal)
}

/// The skills a marketplace file lists: `(group, folder)` pairs, folders without `./`.
pub(in crate::chat::automation) fn skill_paths(marketplace: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for plugin in marketplace
        .get("plugins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let group = plugin
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("skills")
            .to_string();
        let base = plugin
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("./"))
            .unwrap_or("./")
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        for skill in plugin
            .get("skills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let rel = skill.trim_start_matches("./").trim_end_matches('/');
            if rel.split('/').any(|s| s == ".." || s.is_empty()) {
                continue;
            }
            let full = if base.is_empty() {
                rel.to_string()
            } else {
                format!("{base}/{rel}")
            };
            if !out.iter().any(|(_, p): &(String, String)| *p == full) {
                out.push((group.clone(), full));
            }
        }
    }
    out
}

/// One catalogue's skills, with each skill's name and description read from its SKILL.md.
async fn read_catalog(client: &reqwest::Client, repo: &str) -> Result<Vec<Listing>, ChatError> {
    let mut last = String::new();
    for branch in ["main", "master"] {
        let url = format!(
            "{}/{repo}/{branch}/.claude-plugin/marketplace.json",
            raw_base()
        );
        let text = match get_text(client, &url).await {
            Ok(t) => t,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let market: Value = serde_json::from_str(&text).map_err(|e| {
            ChatError::Invalid(format!("{repo}: the catalogue file is not readable ({e})."))
        })?;
        let paths = skill_paths(&market);
        let reads = paths.iter().map(|(group, p)| {
            let url = format!("{}/{repo}/{branch}/{p}/SKILL.md", raw_base());
            let (group, p) = (group.clone(), p.clone());
            async move {
                let text = get_text(client, &url).await.ok()?;
                let (name, description, _) = crate::chat::skills::parse(&text)?;
                Some(Listing {
                    catalog: repo.to_string(),
                    branch: branch.to_string(),
                    path: p,
                    name,
                    description,
                    group,
                })
            }
        });
        let mut found: Vec<Listing> = futures::future::join_all(reads)
            .await
            .into_iter()
            .flatten()
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        return Ok(found);
    }
    Err(ChatError::Invalid(format!(
        "{repo} has no skill catalogue (.claude-plugin/marketplace.json): {last}"
    )))
}

pub struct Browse {
    pub catalogs: Vec<String>,
    pub listings: Vec<Listing>,
    pub errors: Vec<String>,
}

/// Every catalogue's skills, from the cache when it is recent.
pub(in crate::chat) async fn browse(config_dir: &Path, refresh: bool) -> Result<Browse, ChatError> {
    let mut store = load(config_dir);
    let fresh = store
        .fetched_at
        .is_some_and(|t| chrono::Utc::now() - t < chrono::Duration::hours(CACHE_HOURS));
    if fresh && !refresh && !store.listings.is_empty() {
        return Ok(Browse {
            catalogs: store.catalogs,
            listings: store.listings,
            errors: Vec::new(),
        });
    }
    let client = client()?;
    let mut listings = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for repo in &store.catalogs {
        match read_catalog(&client, repo).await {
            Ok(l) => listings.extend(l),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if !listings.is_empty() {
        store.listings = listings.clone();
        store.fetched_at = Some(chrono::Utc::now());
        save(config_dir, &store)?;
    }
    Ok(Browse {
        catalogs: store.catalogs,
        listings,
        errors,
    })
}

pub(in crate::chat) async fn add_catalog(config_dir: &Path, input: &str) -> Result<(), ChatError> {
    let repo = repo_of(input)
        .ok_or_else(|| ChatError::Invalid("Give a GitHub repository, like owner/repo.".into()))?;
    read_catalog(&client()?, &repo).await?;
    let mut store = load(config_dir);
    if !store.catalogs.contains(&repo) {
        store.catalogs.push(repo);
    }
    store.fetched_at = None;
    save(config_dir, &store)
}

pub(in crate::chat) fn remove_catalog(config_dir: &Path, repo: &str) -> Result<(), ChatError> {
    let mut store = load(config_dir);
    store.catalogs.retain(|c| c != repo);
    store.listings.retain(|l| l.catalog != repo);
    save(config_dir, &store)
}

/// The files under `folder` in a GitHub tree listing, relative to it.
pub(in crate::chat::automation) fn files_in(tree: &Value, folder: &str) -> Vec<String> {
    let prefix = format!("{}/", folder.trim_end_matches('/'));
    tree.get("tree")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|e| e.get("path").and_then(Value::as_str))
        .filter_map(|p| p.strip_prefix(&prefix))
        .filter(|rel| *rel != "SKILL.md" && !rel.split('/').any(|s| s.starts_with('.')))
        .map(str::to_string)
        .collect()
}

pub struct Installed {
    pub name: String,
    pub files: usize,
    /// Why the other files were not fetched, when they were not.
    pub note: Option<String>,
}

/// Adds a catalogue skill as one of the user's: its instructions, then its files.
pub(crate) async fn install(
    config_dir: &Path,
    catalog: &str,
    branch: &str,
    folder: &str,
) -> Result<Installed, ChatError> {
    let repo = repo_of(catalog).ok_or_else(|| ChatError::Invalid("Not a catalogue.".into()))?;
    if folder.split('/').any(|s| s == ".." || s.is_empty()) || !matches!(branch, "main" | "master")
    {
        return Err(ChatError::Invalid("Not a skill of that catalogue.".into()));
    }
    let client = client()?;
    let text = get_text(
        &client,
        &format!("{}/{repo}/{branch}/{folder}/SKILL.md", raw_base()),
    )
    .await?;
    let (name, description, body) = crate::chat::skills::parse(&text)
        .ok_or_else(|| ChatError::Invalid("That skill has no name and description.".into()))?;
    crate::chat::skills::save(config_dir, &name, &description, &body)?;
    // Its scripts, references and templates: listed by GitHub's API, fetched from raw files.
    let api =
        std::env::var("CORTEX_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    let tree = match client
        .get(format!("{api}/repos/{repo}/git/trees/{branch}?recursive=1"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
        _ => None,
    };
    let Some(tree) = tree else {
        return Ok(Installed {
            name,
            files: 0,
            note: Some(
                "GitHub did not list the skill's folder, so only its instructions were added."
                    .into(),
            ),
        });
    };
    let mut files = 0;
    let mut skipped: Vec<String> = Vec::new();
    for rel in files_in(&tree, folder).into_iter().take(MAX_FILES) {
        let url = format!("{}/{repo}/{branch}/{folder}/{rel}", raw_base());
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.bytes().await {
                Ok(b) => match crate::chat::skills::add_file(config_dir, &name, &rel, &b) {
                    Ok(()) => files += 1,
                    Err(e) => skipped.push(e.to_string()),
                },
                Err(e) => skipped.push(e.to_string()),
            },
            _ => skipped.push(rel),
        }
    }
    let note = (!skipped.is_empty()).then(|| {
        format!(
            "{} files were left out ({}).",
            skipped.len(),
            skipped
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    Ok(Installed { name, files, note })
}

/// Publishes skills as a catalogue: writes `.claude-plugin/marketplace.json` and
/// `skills/<name>/` (SKILL.md and its files) into `dest`, ready to push to a GitHub repository
/// that others add by `owner/repo`. Returns how many skills were written.
pub(crate) fn export(
    config_dir: &Path,
    names: &[String],
    dest: &Path,
    catalog_name: &str,
    author: &str,
) -> Result<usize, ChatError> {
    if names.is_empty() {
        return Err(ChatError::Invalid("Choose at least one skill.".into()));
    }
    let skills_root = dest.join("skills");
    std::fs::create_dir_all(dest.join(".claude-plugin")).map_err(ChatError::internal)?;
    let mut paths = Vec::new();
    for name in names {
        let skill = crate::chat::skills::find(config_dir, name)
            .ok_or_else(|| format!("There is no skill named {name}."))
            .map_err(ChatError::Invalid)?;
        let dir = skills_root.join(&skill.name);
        std::fs::create_dir_all(&dir).map_err(ChatError::internal)?;
        std::fs::write(
            dir.join("SKILL.md"),
            crate::chat::skills::render(&skill.name, &skill.description, &skill.body),
        )
        .map_err(ChatError::internal)?;
        for rel in &skill.files {
            // Whole and exact (binaries too): read from the skill's folder, decrypted if kept sealed.
            let bytes = crate::chat::at_rest::read(
                crate::chat::skills::skills_dir(config_dir)
                    .join(&skill.name)
                    .join(rel),
            )
            .map_err(ChatError::internal)?;
            let target = dir.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(ChatError::internal)?;
            }
            std::fs::write(target, bytes).map_err(ChatError::internal)?;
        }
        paths.push(format!("./skills/{}", skill.name));
    }
    let slug: String = catalog_name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = if slug.trim_matches('-').is_empty() {
        "my-skills".to_string()
    } else {
        slug.trim_matches('-').to_string()
    };
    let market = serde_json::json!({
        "name": slug,
        "owner": { "name": if author.trim().is_empty() { "Cortex user" } else { author.trim() } },
        "metadata": { "description": format!("Skills shared from Cortex: {}", names.join(", ")), "version": "1.0.0" },
        "plugins": [{ "name": slug, "description": catalog_name.trim(), "source": "./", "strict": false, "skills": paths }]
    });
    std::fs::write(
        dest.join(".claude-plugin").join("marketplace.json"),
        serde_json::to_string_pretty(&market).map_err(ChatError::internal)?,
    )
    .map_err(ChatError::internal)?;
    let readme = format!("# {}\n\nSkills shared from Cortex. Add this catalogue in Cortex (Skills → Import → Browse skill catalogues → Add catalogue, then `owner/repo`), or with any app that reads `.claude-plugin/marketplace.json`.\n\n{}\n", catalog_name.trim(), names.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n"));
    if !dest.join("README.md").exists() {
        std::fs::write(dest.join("README.md"), readme).map_err(ChatError::internal)?;
    }
    Ok(names.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalogues_and_folders_are_read() {
        let market = json!({"plugins": [
            {"name": "document-skills", "source": "./", "skills": ["./skills/pdf", "./skills/docx/"]},
            {"name": "more", "source": "./extra", "skills": ["./a", "../escape"]}
        ]});
        assert_eq!(
            skill_paths(&market),
            vec![
                ("document-skills".into(), "skills/pdf".into()),
                ("document-skills".into(), "skills/docx".into()),
                ("more".into(), "extra/a".into())
            ]
        );
        assert_eq!(
            repo_of("https://github.com/anthropics/skills/").as_deref(),
            Some("anthropics/skills")
        );
        assert_eq!(
            repo_of("anthropics/skills").as_deref(),
            Some("anthropics/skills")
        );
        assert_eq!(repo_of("not a repo"), None);
        let tree = json!({"tree": [
            {"path": "skills/pdf/SKILL.md", "type": "blob"},
            {"path": "skills/pdf/scripts/fill.py", "type": "blob"},
            {"path": "skills/pdf/scripts", "type": "tree"},
            {"path": "skills/pdf/.hidden", "type": "blob"},
            {"path": "skills/pdfx/other.md", "type": "blob"}
        ]});
        assert_eq!(
            files_in(&tree, "skills/pdf"),
            vec!["scripts/fill.py".to_string()]
        );
    }

    #[test]
    fn exported_catalogues_read_back() {
        let dir = tempfile::tempdir().unwrap();
        crate::chat::skills::save(
            dir.path(),
            "trip-planner",
            "Use when planning a trip.",
            "1. Ask the dates.",
        )
        .unwrap();
        crate::chat::skills::add_file(dir.path(), "trip-planner", "templates/day.md", b"# Day")
            .unwrap();
        let out = dir.path().join("published");
        assert_eq!(
            export(
                dir.path(),
                &["trip-planner".into()],
                &out,
                "Travel skills",
                "Ana"
            )
            .unwrap(),
            1
        );
        let market: Value = serde_json::from_str(
            &std::fs::read_to_string(out.join(".claude-plugin/marketplace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            skill_paths(&market),
            vec![("travel-skills".into(), "skills/trip-planner".into())]
        );
        let md = std::fs::read_to_string(out.join("skills/trip-planner/SKILL.md")).unwrap();
        assert_eq!(crate::chat::skills::parse(&md).unwrap().0, "trip-planner");
        assert_eq!(
            std::fs::read_to_string(out.join("skills/trip-planner/templates/day.md")).unwrap(),
            "# Day"
        );
        assert!(export(dir.path(), &[], &out, "x", "").is_err());
    }

    /// Against the real public catalogue (`CORTEX_TEST_NETWORK=1`).
    #[tokio::test]
    #[ignore]
    async fn anthropics_catalogue_is_browsed_and_a_skill_added() {
        if std::env::var("CORTEX_TEST_NETWORK").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let b = browse(dir.path(), true).await.unwrap();
        assert!(b.listings.len() >= 10, "{:?}", b.errors);
        let pdf = b
            .listings
            .iter()
            .find(|l| l.path == "skills/pdf")
            .expect("pdf skill listed");
        let done = install(dir.path(), &pdf.catalog, &pdf.branch, &pdf.path)
            .await
            .unwrap();
        assert_eq!(done.name, pdf.name);
        assert!(crate::chat::skills::find(dir.path(), &done.name).is_some());
        eprintln!(
            "listed {}, added {} with {} files ({:?})",
            b.listings.len(),
            done.name,
            done.files,
            done.note
        );
    }
}
