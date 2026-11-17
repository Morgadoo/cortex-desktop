//! Keyless web search for the `web_search` tool: Bing's HTML results page, with DuckDuckGo's as
//! a fallback. Both are parsed with `scraper`; neither needs an account or an API key.

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
    String::from_utf8(bytes).ok().filter(|u| u.starts_with("http"))
}

pub fn parse_bing(html: &str) -> Vec<Hit> {
    let doc = Html::parse_document(html);
    let item = Selector::parse("li.b_algo").expect("selector");
    let link = Selector::parse("h2 a").expect("selector");
    let snippet = Selector::parse(".b_caption p, p.b_lineclamp2, p.b_lineclamp3, p.b_lineclamp4, p").expect("selector");
    doc.select(&item)
        .filter_map(|li| {
            let a = li.select(&link).next()?;
            let url = unwrap_bing(a.value().attr("href")?)?;
            let title = text_of(a);
            let snippet = li.select(&snippet).next().map(text_of).unwrap_or_default();
            (!title.is_empty()).then_some(Hit { title, url, snippet })
        })
        .collect()
}

/// DuckDuckGo wraps result links as `//duckduckgo.com/l/?uddg=<percent-encoded target>`.
fn unwrap_ddg(href: &str) -> Option<String> {
    let absolute = if href.starts_with("//") { format!("https:{href}") } else { href.to_string() };
    let parsed = url::Url::parse(&absolute).ok()?;
    match parsed.query_pairs().find(|(k, _)| k == "uddg") {
        Some((_, target)) => Some(target.into_owned()),
        None => Some(absolute),
    }
}

pub fn parse_ddg(html: &str) -> Vec<Hit> {
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
            (!title.is_empty() && !url.contains("duckduckgo.com/y.js")).then_some(Hit { title, url, snippet })
        })
        .collect()
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

/// Up to `max` results for `query`, from the first engine that answers with any.
pub async fn search(query: &str, max: usize) -> Result<Vec<Hit>, String> {
    let client = client()?;
    let mut errors = Vec::new();

    match client
        .get("https://www.bing.com/search")
        .query(&[("q", query), ("setlang", "en"), ("count", "15")])
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
        .form(&[("q", query)])
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

    Err(format!("Web search failed: {}", errors.join("; ")))
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
        let target = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://www.rust-lang.org/");
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
    fn ddg_results_are_parsed_and_unwrapped() {
        let html = r#"<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frig.rs%2Fdocs&amp;rut=1">Rig docs</a>
            <a class="result__snippet">Build LLM apps in Rust.</a></div>"#;
        let hits = parse_ddg(html);
        assert_eq!(hits, vec![Hit { title: "Rig docs".into(), url: "https://rig.rs/docs".into(), snippet: "Build LLM apps in Rust.".into() }]);
    }

    #[test]
    fn duplicates_and_extras_are_dropped() {
        let hit = |u: &str| Hit { title: "t".into(), url: u.into(), snippet: String::new() };
        let hits = dedup(vec![hit("https://a/"), hit("https://a"), hit("https://b"), hit("https://c")], 2);
        assert_eq!(hits.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(), ["https://a/", "https://b"]);
    }
}

#[cfg(test)]
mod live {
    /// Hits the network; run with `cargo test -- --ignored live_search`.
    #[tokio::test]
    #[ignore]
    async fn live_search() {
        let hits = super::search("rig rust llm library", 5).await.expect("search");
        assert!(!hits.is_empty());
        for h in &hits {
            println!("HIT {} | {} | {}", h.title, h.url, h.snippet.chars().take(60).collect::<String>());
        }
    }
}
