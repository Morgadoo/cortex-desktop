//! Community agents (LobeHub's and LibreChat's agent marketplaces): browse a public catalog of
//! agents others wrote and add one as an agent of your own, to use and change as you like.
//!
//! The catalog is LobeHub's open agent index (`chat-agents.lobehub.com`, MIT, about 500 agents),
//! or any catalog with the same layout (Settings can point at another). An agent from it is a
//! prompt, an avatar and starter questions: no code runs, and the agent gets Cortex's tools under
//! your settings like any agent you write.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_SOURCE: &str = "https://chat-agents.lobehub.com";
/// How long the catalog is kept before it is fetched again.
const CACHE_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub avatar: String,
    pub category: String,
    pub tags: Vec<String>,
    pub author: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    source: String,
    language: String,
    fetched_at: chrono::DateTime<chrono::Utc>,
    listings: Vec<Listing>,
}

fn cache_path(config_dir: &Path) -> PathBuf {
    super::store::chat_dir(config_dir).join("agent-market.json")
}

/// The catalog's index in LobeHub's layout, as listings.
pub fn parse_index(body: &Value) -> Vec<Listing> {
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    body.get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .filter_map(|a| {
                    let meta = a.get("meta")?;
                    let identifier = s(a, "identifier");
                    if identifier.is_empty()
                        || !identifier
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        return None;
                    }
                    Some(Listing {
                        identifier,
                        title: s(meta, "title"),
                        description: s(meta, "description"),
                        avatar: s(meta, "avatar"),
                        category: s(meta, "category"),
                        tags: meta
                            .get("tags")
                            .and_then(Value::as_array)
                            .map(|t| {
                                t.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        author: s(a, "author"),
                        created_at: s(a, "createdAt"),
                    })
                })
                .filter(|l| !l.title.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The catalog's language suffix: English unless the catalog has the app's language.
fn locale(language: &str) -> &'static str {
    match language.split('-').next().unwrap_or_default() {
        "pt" => "pt-BR",
        "es" => "es-ES",
        "fr" => "fr-FR",
        "de" => "de-DE",
        "ar" => "ar",
        "zh" => "zh-CN",
        "ja" => "ja-JP",
        _ => "en-US",
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch_json(url: &str) -> Result<Value, String> {
    let r = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("The agent catalog did not answer: {e}"))?;
    if !r.status().is_success() {
        return Err(format!("The agent catalog answered {}.", r.status()));
    }
    r.json()
        .await
        .map_err(|e| format!("The agent catalog sent something unreadable: {e}"))
}

/// The catalog, from the cache when it is recent.
pub async fn listings(
    config_dir: &Path,
    source: &str,
    language: &str,
    refresh: bool,
) -> Result<Vec<Listing>, String> {
    let source = source.trim().trim_end_matches('/');
    let lang = locale(language);
    if !refresh {
        if let Some(c) = super::store::read_json::<Cache>(&cache_path(config_dir)) {
            if c.source == source
                && c.language == lang
                && chrono::Utc::now() - c.fetched_at < chrono::Duration::hours(CACHE_HOURS)
            {
                return Ok(c.listings);
            }
        }
    }
    let body = match fetch_json(&format!("{source}/index.{lang}.json")).await {
        Ok(b) => b,
        // Not every catalog is translated.
        Err(_) if lang != "en-US" => fetch_json(&format!("{source}/index.en-US.json")).await?,
        Err(e) => return Err(e),
    };
    let listings = parse_index(&body);
    if listings.is_empty() {
        return Err("The agent catalog lists no agents.".into());
    }
    let _ = super::store::write_json(
        &cache_path(config_dir),
        &Cache {
            source: source.into(),
            language: lang.into(),
            fetched_at: chrono::Utc::now(),
            listings: listings.clone(),
        },
    );
    Ok(listings)
}

/// A catalog agent as one of yours: its prompt, avatar, description and opening questions.
pub fn to_agent(body: &Value, id: String) -> Result<super::agents::Agent, String> {
    let s = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let meta = body.get("meta");
    let name = s(meta.and_then(|m| m.get("title")));
    let instructions = s(body.pointer("/config/systemRole"));
    if name.is_empty() || instructions.is_empty() {
        return Err("That agent has no name or instructions.".into());
    }
    let mut starters: Vec<String> = body
        .get("openingQuestions")
        .and_then(Value::as_array)
        .map(|q| {
            q.iter()
                .filter_map(Value::as_str)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .take(4)
                .collect()
        })
        .unwrap_or_default();
    if starters.is_empty() {
        starters = body
            .get("examples")
            .and_then(Value::as_array)
            .map(|ex| {
                ex.iter()
                    .filter(|e| e.get("role").and_then(Value::as_str) == Some("user"))
                    .filter_map(|e| e.get("content").and_then(Value::as_str))
                    .map(|t| t.chars().take(200).collect())
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut instructions = instructions;
    if let Some(opening) = body
        .get("openingMessage")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
    {
        instructions.push_str(&format!(
            "\n\nWhen a conversation starts, you may open with: {}",
            opening.trim()
        ));
    }
    let avatar = s(meta.and_then(|m| m.get("avatar")));
    Ok(super::agents::Agent {
        id,
        name: name.chars().take(80).collect(),
        // Emoji avatars carry over; image links do not (the app shows no remote images here).
        avatar: if avatar.starts_with("http") {
            String::new()
        } else {
            avatar.chars().take(8).collect()
        },
        description: s(meta.and_then(|m| m.get("description")))
            .chars()
            .take(400)
            .collect(),
        instructions,
        starters,
        ..Default::default()
    })
}

/// Adds a catalog agent as one of yours; returns its new id.
pub async fn install(
    config_dir: &Path,
    source: &str,
    language: &str,
    identifier: &str,
) -> Result<super::agents::Agent, String> {
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Not an agent of the catalog.".into());
    }
    let source = source.trim().trim_end_matches('/');
    let lang = locale(language);
    let body = match fetch_json(&format!("{source}/{identifier}.{lang}.json")).await {
        Ok(b) => b,
        Err(_) if lang != "en-US" => {
            fetch_json(&format!("{source}/{identifier}.en-US.json")).await?
        }
        Err(e) => return Err(e),
    };
    let mut agent = to_agent(&body, uuid::Uuid::new_v4().to_string())?;
    // A name already taken gets a number, as imports do.
    let taken: Vec<String> = super::agents::load(config_dir)
        .into_iter()
        .map(|a| a.name)
        .collect();
    if taken.contains(&agent.name) {
        let base = agent.name.clone();
        agent.name = (2..)
            .map(|n| format!("{base} ({n})"))
            .find(|n| !taken.contains(n))
            .unwrap_or(base);
    }
    super::agents::save(config_dir, agent.clone())?;
    Ok(agent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_index_and_an_agent_are_read() {
        let index = json!({"schemaVersion": 1, "agents": [
            {"identifier": "travel-guide", "author": "a", "createdAt": "2025-01-02", "meta": {"title": "Travel Guide", "description": "Plans trips", "avatar": "🧭", "category": "life", "tags": ["travel"]}},
            {"identifier": "../evil", "meta": {"title": "Bad"}},
            {"identifier": "untitled", "meta": {"title": " "}}
        ]});
        let l = parse_index(&index);
        assert_eq!(l.len(), 1);
        assert_eq!(
            (
                l[0].identifier.as_str(),
                l[0].avatar.as_str(),
                l[0].tags[0].as_str()
            ),
            ("travel-guide", "🧭", "travel")
        );
        let body = json!({"meta": {"title": "Travel Guide", "avatar": "🧭", "description": "Plans trips"}, "config": {"systemRole": "You plan trips."}, "openingMessage": "Where to?", "openingQuestions": ["A weekend in Porto", " "]});
        let a = to_agent(&body, "00000000-0000-4000-8000-000000000001".into()).unwrap();
        assert_eq!(a.name, "Travel Guide");
        assert!(
            a.instructions.starts_with("You plan trips.") && a.instructions.contains("Where to?")
        );
        assert_eq!(a.starters, vec!["A weekend in Porto"]);
        assert!(to_agent(&json!({"meta": {"title": "X"}}), "x".into()).is_err());
        assert_eq!(locale("pt"), "pt-BR");
        assert_eq!(locale("xx"), "en-US");
    }

    #[tokio::test]
    #[ignore]
    async fn the_public_catalog_answers() {
        let dir = tempfile::tempdir().unwrap();
        let l = listings(dir.path(), DEFAULT_SOURCE, "en", true)
            .await
            .unwrap();
        assert!(l.len() > 100);
        let agent = install(dir.path(), DEFAULT_SOURCE, "en", &l[0].identifier)
            .await
            .unwrap();
        assert!(!agent.instructions.is_empty());
    }
}
