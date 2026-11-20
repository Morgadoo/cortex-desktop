//! Web search for the `web_search` tool.
//!
//! Keyless by default: Bing's HTML results page, with DuckDuckGo's as a fallback, both parsed
//! with `scraper`. Settings can name another engine, as Open WebUI and Onyx allow: a SearXNG
//! instance (self-hosted, private), Brave Search or Tavily (API keys in the keychain). When the
//! chosen engine fails, the keyless search answers instead.

use crate::chat::error::ChatError;
use std::path::Path;

use crate::support::settings_file::SettingsFile;
use std::time::Duration;

use base64::Engine;
use scraper::{Html, Selector};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/130.0 Safari/537.36";

fn text_of(element: scraper::ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bing wraps result links as `/ck/a?...&u=a1<base64url of the target>`.
fn unwrap_bing(href: &str) -> Option<String> {
    let parsed = url::Url::parse(href).ok()?;
    if !parsed.host_str().is_some_and(|h| h.ends_with("bing.com")) {
        return Some(href.to_string());
    }
    let encoded = parsed.query_pairs().find(|(k, _)| k == "u")?.1.into_owned();
    let encoded = encoded.strip_prefix("a1").unwrap_or(&encoded);
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .ok()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|u| u.starts_with("http"))
}

pub(in crate::chat::grounding) fn parse_bing(html: &str) -> Vec<Hit> {
    let doc = Html::parse_document(html);
    let item = Selector::parse("li.b_algo").expect("selector");
    let link = Selector::parse("h2 a").expect("selector");
    let snippet =
        Selector::parse(".b_caption p, p.b_lineclamp2, p.b_lineclamp3, p.b_lineclamp4, p")
            .expect("selector");
    doc.select(&item)
        .filter_map(|li| {
            let a = li.select(&link).next()?;
            let url = unwrap_bing(a.value().attr("href")?)?;
            let title = text_of(a);
            let snippet = li.select(&snippet).next().map(text_of).unwrap_or_default();
            (!title.is_empty()).then_some(Hit {
                title,
                url,
                snippet,
            })
        })
        .collect()
}

/// DuckDuckGo wraps result links as `//duckduckgo.com/l/?uddg=<percent-encoded target>`.
fn unwrap_ddg(href: &str) -> Option<String> {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    let parsed = url::Url::parse(&absolute).ok()?;
    match parsed.query_pairs().find(|(k, _)| k == "uddg") {
        Some((_, target)) => Some(target.into_owned()),
        None => Some(absolute),
    }
}

pub(in crate::chat::grounding) fn parse_ddg(html: &str) -> Vec<Hit> {
    let doc = Html::parse_document(html);
    let item = Selector::parse(".result").expect("selector");
    let link = Selector::parse("a.result__a").expect("selector");
    let snippet = Selector::parse(".result__snippet").expect("selector");
    doc.select(&item)
        .filter_map(|r| {
            let a = r.select(&link).next()?;
            let url = unwrap_ddg(a.value().attr("href")?)?;
            let title = text_of(a);
            let snippet = r.select(&snippet).next().map(text_of).unwrap_or_default();
            (!title.is_empty() && !url.contains("duckduckgo.com/y.js")).then_some(Hit {
                title,
                url,
                snippet,
            })
        })
        .collect()
}

fn client() -> Result<reqwest::Client, ChatError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(ChatError::internal)
}

pub const SECRET: &str = "web-search";
pub const ENGINES: &[&str] = &["auto", "searxng", "brave", "tavily"];

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchSettings {
    /// `auto` (keyless), `searxng`, `brave` or `tavily`.
    pub engine: String,
    /// SearXNG: the instance's address.
    pub base_url: String,
    /// Whether a key is saved in the keychain (never the key itself).
    pub has_key: bool,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            engine: "auto".into(),
            base_url: String::new(),
            has_key: false,
        }
    }
}

const SEARCH_FILE: SettingsFile<SearchSettings> = SettingsFile::sealed("chat/search.json");

pub(in crate::chat) fn load_settings(config_dir: &Path) -> SearchSettings {
    SEARCH_FILE.load(config_dir)
}

pub(in crate::chat) fn save_settings(
    config_dir: &std::path::Path,
    settings: &SearchSettings,
) -> Result<(), ChatError> {
    let mut s = settings.clone();
    if !ENGINES.contains(&s.engine.as_str()) {
        s.engine = "auto".into();
    }
    s.base_url = s.base_url.trim().trim_end_matches('/').to_string();
    SEARCH_FILE
        .save(config_dir, &s)
        .map_err(String::from)
        .map_err(ChatError::Invalid)
}

async fn key() -> Result<Option<String>, ChatError> {
    crate::chat::secrets::local()?
        .load(SECRET)
        .await
        .map_err(ChatError::Invalid)
}

/// SearXNG's JSON results (`format=json` must be enabled on the instance).
pub(in crate::chat::grounding) fn parse_searxng(body: &serde_json::Value) -> Vec<Hit> {
    hits_from(body.get("results"), "title", "url", "content")
}

/// Brave Search API: `web.results`.
pub(in crate::chat::grounding) fn parse_brave(body: &serde_json::Value) -> Vec<Hit> {
    hits_from(body.pointer("/web/results"), "title", "url", "description")
}

/// Tavily: `results`.
pub(in crate::chat::grounding) fn parse_tavily(body: &serde_json::Value) -> Vec<Hit> {
    hits_from(body.get("results"), "title", "url", "content")
}

fn hits_from(list: Option<&serde_json::Value>, title: &str, url: &str, snippet: &str) -> Vec<Hit> {
    let field = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    list.and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|v| Hit {
                    title: strip_tags(&field(v, title)),
                    url: field(v, url),
                    snippet: strip_tags(&field(v, snippet)),
                })
                .filter(|h| h.url.starts_with("http") && !h.title.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Brave marks matches with `<strong>`; plain text reads better in a tool result.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Searches with the engine chosen in Settings, or the keyless one; a failing engine falls back.
/// How recent results must be (Open WebUI's and Onyx's time filters): `day`, `week`, `month`
/// or `year`; anything else means any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recency {
    Day,
    Week,
    Month,
    Year,
}

impl Recency {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "day" | "24h" | "today" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            "year" => Some(Self::Year),
            _ => None,
        }
    }

    /// The word SearXNG and Tavily use.
    fn word(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    /// Brave's `freshness`.
    fn brave(self) -> &'static str {
        match self {
            Self::Day => "pd",
            Self::Week => "pw",
            Self::Month => "pm",
            Self::Year => "py",
        }
    }

    /// DuckDuckGo's `df`.
    fn ddg(self) -> &'static str {
        match self {
            Self::Day => "d",
            Self::Week => "w",
            Self::Month => "m",
            Self::Year => "y",
        }
    }

    /// Bing's `filters` (it has no past-year filter; a year leaves results unfiltered there).
    fn bing(self) -> Option<&'static str> {
        match self {
            Self::Day => Some("ex1:\"ez1\""),
            Self::Week => Some("ex1:\"ez2\""),
            Self::Month => Some("ex1:\"ez3\""),
            Self::Year => None,
        }
    }
}

/// Domains to keep results to, from what a model sent: bare hosts like `docs.rs`, lowercase,
/// without scheme, `www.` or path; at most 5.
pub(in crate::chat) fn clean_sites(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        let v = v.trim().to_lowercase();
        let v = v.trim_start_matches("site:");
        let v = v.split("://").last().unwrap_or_default();
        let host = v
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .trim_start_matches("www.")
            .trim_end_matches('.');
        if host.contains('.')
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && !out.iter().any(|h| h == host)
        {
            out.push(host.to_string());
        }
    }
    out.truncate(5);
    out
}

/// The query with `site:` operators, which Bing, DuckDuckGo, SearXNG and Brave understand.
pub(in crate::chat::grounding) fn with_sites(query: &str, sites: &[String]) -> String {
    match sites {
        [] => query.to_string(),
        [one] => format!("{query} site:{one}"),
        many => format!(
            "{query} ({})",
            many.iter()
                .map(|s| format!("site:{s}"))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    }
}

/// Whether a URL is on one of the sites (the site itself or a subdomain of it).
pub(in crate::chat::grounding) fn on_sites(url: &str, sites: &[String]) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    sites.is_empty()
        || sites
            .iter()
            .any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Searches only the given sites. Results from other sites are dropped, so an engine that
/// ignores `site:` gives fewer results, never wrong ones.
pub(in crate::chat) async fn search_sites(
    config_dir: &std::path::Path,
    query: &str,
    max: usize,
    recency: Option<Recency>,
    sites: &[String],
) -> Result<Vec<Hit>, ChatError> {
    if sites.is_empty() {
        return search_with(config_dir, query, max, recency).await;
    }
    let settings = load_settings(config_dir);
    let hits = if settings.engine == "tavily" {
        match tavily(query, (max * 2).min(20), recency, sites).await {
            Ok(hits) => hits,
            Err(e) => {
                log::warn!("tavily search failed: {e}");
                search(&with_sites(query, sites), 15, recency).await?
            }
        }
    } else {
        search_with(
            config_dir,
            &with_sites(query, sites),
            (max * 2).min(20),
            recency,
        )
        .await?
    };
    Ok(hits
        .into_iter()
        .filter(|h| on_sites(&h.url, sites))
        .take(max)
        .collect())
}

pub(in crate::chat::grounding) async fn search_with(
    config_dir: &std::path::Path,
    query: &str,
    max: usize,
    recency: Option<Recency>,
) -> Result<Vec<Hit>, ChatError> {
    let settings = load_settings(config_dir);
    if settings.engine == "auto" {
        return search(query, max, recency).await;
    }
    match engine_search(&settings, query, max, recency).await {
        Ok(hits) if !hits.is_empty() => Ok(dedup(hits, max)),
        Ok(_) => search(query, max, recency).await,
        Err(e) => {
            log::warn!(
                "{} search failed, using the keyless search: {e}",
                settings.engine
            );
            search(query, max, recency).await
        }
    }
}

async fn engine_search(
    settings: &SearchSettings,
    query: &str,
    max: usize,
    recency: Option<Recency>,
) -> Result<Vec<Hit>, ChatError> {
    let client = client()?;
    let json = |r: reqwest::Response| async move {
        let status = r.status();
        if !status.is_success() {
            return Err(ChatError::Invalid(format!("HTTP {status}")));
        }
        r.json::<serde_json::Value>()
            .await
            .map_err(ChatError::internal)
    };
    match settings.engine.as_str() {
        "searxng" => {
            if settings.base_url.is_empty() {
                return Err(ChatError::Invalid("no SearXNG address".into()));
            }
            let mut params = vec![("q", query), ("format", "json")];
            if let Some(r) = recency {
                params.push(("time_range", r.word()));
            }
            let r = client
                .get(format!("{}/search", settings.base_url))
                .query(&params)
                .send()
                .await
                .map_err(ChatError::internal)?;
            Ok(parse_searxng(&json(r).await?))
        }
        "brave" => {
            let key = key()
                .await?
                .ok_or_else(|| ChatError::Invalid("no Brave Search key".into()))?;
            let r = client
                .get("https://api.search.brave.com/res/v1/web/search")
                .query(&[("q", query), ("count", &max.min(20).to_string())])
                .query(
                    &recency
                        .map(|r| vec![("freshness", r.brave())])
                        .unwrap_or_default(),
                )
                .header("X-Subscription-Token", key)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(ChatError::internal)?;
            Ok(parse_brave(&json(r).await?))
        }
        "tavily" => tavily(query, max, recency, &[]).await,
        other => Err(ChatError::Invalid(format!("unknown engine {other}"))),
    }
}

/// Tavily, which takes the sites as a list instead of `site:` operators.
async fn tavily(
    query: &str,
    max: usize,
    recency: Option<Recency>,
    sites: &[String],
) -> Result<Vec<Hit>, ChatError> {
    let key = key()
        .await?
        .ok_or_else(|| ChatError::Invalid("no Tavily key".into()))?;
    let mut body = serde_json::json!({ "query": query, "max_results": max.min(20) });
    if let Some(r) = recency {
        body["time_range"] = r.word().into();
    }
    if !sites.is_empty() {
        body["include_domains"] = sites.into();
    }
    let r = client()?
        .post("https://api.tavily.com/search")
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(ChatError::internal)?;
    let status = r.status();
    if !status.is_success() {
        return Err(ChatError::Invalid(format!("HTTP {status}")));
    }
    Ok(parse_tavily(
        &r.json::<serde_json::Value>()
            .await
            .map_err(ChatError::internal)?,
    ))
}

/// Up to `max` results for `query`, from the first engine that answers with any.
pub(in crate::chat::grounding) async fn search(
    query: &str,
    max: usize,
    recency: Option<Recency>,
) -> Result<Vec<Hit>, ChatError> {
    let client = client()?;
    let mut errors = Vec::new();

    match client
        .get("https://www.bing.com/search")
        .query(&[("q", query), ("setlang", "en"), ("count", "15")])
        .query(
            &recency
                .and_then(Recency::bing)
                .map(|f| vec![("filters", f)])
                .unwrap_or_default(),
        )
        .send()
        .await
    {
        Ok(response) => {
            let hits = parse_bing(&response.text().await.unwrap_or_default());
            if !hits.is_empty() {
                return Ok(dedup(hits, max));
            }
            errors.push("Bing returned no results".to_string());
        }
        Err(e) => errors.push(format!("Bing: {e}")),
    }

    match client
        .post("https://html.duckduckgo.com/html/")
        .form(&match recency {
            Some(r) => vec![("q", query), ("df", r.ddg())],
            None => vec![("q", query)],
        })
        .send()
        .await
    {
        Ok(response) => {
            let hits = parse_ddg(&response.text().await.unwrap_or_default());
            if !hits.is_empty() {
                return Ok(dedup(hits, max));
            }
            errors.push("DuckDuckGo returned no results".to_string());
        }
        Err(e) => errors.push(format!("DuckDuckGo: {e}")),
    }

    Err(ChatError::Invalid(format!(
        "Web search failed: {}",
        errors.join("; ")
    )))
}

fn dedup(hits: Vec<Hit>, max: usize) -> Vec<Hit> {
    let mut seen = std::collections::HashSet::new();
    hits.into_iter()
        .filter(|h| seen.insert(h.url.trim_end_matches('/').to_string()))
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bing_results_are_parsed_and_unwrapped() {
        let target =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://www.rust-lang.org/");
        let html = format!(
            r#"<ol id="b_results">
              <li class="b_algo"><h2><a href="https://www.bing.com/ck/a?!&amp;&amp;p=x&amp;u=a1{target}&amp;ntb=1">Rust <strong>Programming</strong> Language</a></h2>
                <div class="b_caption"><p>A language empowering everyone.</p></div></li>
              <li class="b_algo"><h2><a href="https://docs.rs/">Docs.rs</a></h2><p>Crate docs</p></li>
              <li class="b_ad"><h2><a href="https://ads.example">Ad</a></h2></li>
            </ol>"#
        );
        let hits = parse_bing(&html);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].snippet, "A language empowering everyone.");
        assert_eq!(hits[1].url, "https://docs.rs/");
    }

    #[test]
    fn api_engines_are_parsed() {
        let searx = serde_json::json!({ "results": [{ "title": "Rig", "url": "https://rig.rs", "content": "Rust LLM library" }, { "title": "", "url": "https://x" }] });
        assert_eq!(
            parse_searxng(&searx),
            vec![Hit {
                title: "Rig".into(),
                url: "https://rig.rs".into(),
                snippet: "Rust LLM library".into()
            }]
        );
        let brave = serde_json::json!({ "web": { "results": [{ "title": "Tauri <strong>2</strong>", "url": "https://tauri.app", "description": "Build <strong>small</strong> apps" }] } });
        assert_eq!(parse_brave(&brave)[0].snippet, "Build small apps");
        assert_eq!(parse_brave(&brave)[0].title, "Tauri 2");
        let tavily = serde_json::json!({ "results": [{ "title": "Docs", "url": "https://docs.rs", "content": "Crates" }] });
        assert_eq!(parse_tavily(&tavily).len(), 1);
        assert!(parse_brave(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn search_settings_fall_back_to_keyless() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(dir.path()).engine, "auto");
        save_settings(
            dir.path(),
            &SearchSettings {
                engine: "bogus".into(),
                base_url: "https://searx.example/ ".into(),
                has_key: false,
            },
        )
        .unwrap();
        let s = load_settings(dir.path());
        assert_eq!(
            (s.engine.as_str(), s.base_url.as_str()),
            ("auto", "https://searx.example")
        );
    }

    #[test]
    fn ddg_results_are_parsed_and_unwrapped() {
        let html = r#"<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frig.rs%2Fdocs&amp;rut=1">Rig docs</a>
            <a class="result__snippet">Build LLM apps in Rust.</a></div>"#;
        let hits = parse_ddg(html);
        assert_eq!(
            hits,
            vec![Hit {
                title: "Rig docs".into(),
                url: "https://rig.rs/docs".into(),
                snippet: "Build LLM apps in Rust.".into()
            }]
        );
    }

    #[test]
    fn site_filters() {
        let sites = clean_sites(&[
            "https://www.Docs.rs/rig".into(),
            "site:github.com".into(),
            "docs.rs".into(),
            "not a site".into(),
        ]);
        assert_eq!(sites, ["docs.rs", "github.com"]);
        assert_eq!(with_sites("rig", &sites[..1]), "rig site:docs.rs");
        assert_eq!(
            with_sites("rig", &sites),
            "rig (site:docs.rs OR site:github.com)"
        );
        assert!(on_sites("https://docs.rs/rig-core", &sites));
        assert!(on_sites("https://gist.github.com/x", &sites));
        assert!(!on_sites("https://notgithub.com/x", &sites));
        assert!(on_sites("https://anything.example", &[]));
    }

    #[test]
    fn recency_maps_to_each_engine() {
        assert_eq!(Recency::parse("Week"), Some(Recency::Week));
        assert_eq!(Recency::parse("any"), None);
        assert_eq!(
            (
                Recency::Day.word(),
                Recency::Day.brave(),
                Recency::Day.ddg()
            ),
            ("day", "pd", "d")
        );
        assert_eq!(Recency::Month.bing(), Some("ex1:\"ez3\""));
        assert_eq!(Recency::Year.bing(), None, "Bing has no past-year filter");
    }

    #[test]
    fn duplicates_and_extras_are_dropped() {
        let hit = |u: &str| Hit {
            title: "t".into(),
            url: u.into(),
            snippet: String::new(),
        };
        let hits = dedup(
            vec![
                hit("https://a/"),
                hit("https://a"),
                hit("https://b"),
                hit("https://c"),
            ],
            2,
        );
        assert_eq!(
            hits.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(),
            ["https://a/", "https://b"]
        );
    }
}

#[cfg(test)]
mod live {
    /// Hits the network; run with `cargo test -- --ignored live_search`.
    #[tokio::test]
    #[ignore]
    async fn live_search() {
        let query = std::env::var("Q").unwrap_or_else(|_| "rig rust llm library".into());
        let hits = super::search(&query, 10, None).await.expect("search");
        assert!(!hits.is_empty());
        for h in &hits {
            println!(
                "HIT {} | {} | {}",
                h.title,
                h.url,
                crate::support::text::clip(&h.snippet, 60)
            );
        }
    }
}
