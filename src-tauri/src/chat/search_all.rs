//! Search mode (Onyx's): one query over everything the app knows (library passages, knowledge
//! pages, earlier chats) and the web, results first, as cards; an answer from them only when
//! asked for.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// `library`, `knowledge`, `chat` or `web`.
    pub kind: String,
    pub title: String,
    /// Where it is: the library, the chat's date, the site.
    pub place: String,
    pub snippet: String,
    /// What opens it: a URL, a chat id, a knowledge page id, `library:document`.
    pub target: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Results {
    pub cards: Vec<Card>,
    /// What could not be searched, and why.
    pub notes: Vec<String>,
}

/// Readable text for a card: attachment tags, Markdown marks and extra spaces taken out.
fn clip(text: &str, n: usize) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<attachment") {
        plain.push_str(&rest[..start]);
        rest = rest[start..].find('>').map_or("", |end| &rest[start + end + 1..]);
    }
    plain.push_str(rest);
    let plain = plain.replace("</attachment>", " ").replace("**", "").replace("__", "").replace('`', "");
    let flat: String = plain
        .split_whitespace()
        .filter(|w| !w.chars().all(|c| c == '#' || c == '-' || c == '>' || c == '*'))
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() > n { format!("{}…", flat.chars().take(n).collect::<String>()) } else { flat }
}

/// Searches the libraries, knowledge pages and chats, and the web when `web` is set.
pub async fn search(config_dir: &Path, query: &str, web: bool) -> Results {
    let mut out = Results::default();
    let query = query.trim();
    if query.is_empty() {
        return out;
    }
    let (passages, note) = super::embeddings::search(config_dir, &[], query, 6).await;
    if let Some(note) = note {
        out.notes.push(note);
    }
    for p in passages {
        out.cards.push(Card { kind: "library".into(), title: p.document.clone(), place: p.library.clone(), snippet: clip(&p.text, 320), target: format!("{}:{}", p.library, p.document) });
    }
    let pages = super::knowledge::load(config_dir);
    for (page, _) in super::knowledge::rank(&pages, query).into_iter().take(4) {
        out.cards.push(Card { kind: "knowledge".into(), title: page.title.clone(), place: "page".into(), snippet: clip(&page.body, 320), target: page.id.clone() });
    }
    for hit in super::builtins::search_chats(config_dir, query).into_iter().take(6) {
        out.cards.push(Card {
            kind: "chat".into(),
            title: if hit.title.is_empty() { "Untitled".into() } else { hit.title.clone() },
            place: hit.updated_at.format("%Y-%m-%d").to_string(),
            snippet: clip(&hit.snippet, 320),
            target: hit.id.clone(),
        });
    }
    if web {
        match super::search::search_with(config_dir, query, 6, None).await {
            Ok(hits) => {
                for h in hits {
                    let site = h.url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or_default().trim_start_matches("www.").to_string();
                    out.cards.push(Card { kind: "web".into(), title: h.title, place: site, snippet: clip(&h.snippet, 320), target: h.url });
                }
            }
            Err(e) => out.notes.push(format!("The web could not be searched: {e}")),
        }
    }
    out
}

/// The prompt for an answer built only from the cards, citing them by number.
pub fn answer_prompt(query: &str, cards: &[Card]) -> String {
    let sources: Vec<serde_json::Value> = cards
        .iter()
        .enumerate()
        .map(|(i, c)| json!({ "n": i + 1, "kind": c.kind, "title": c.title, "place": c.place, "text": c.snippet }))
        .collect();
    format!(
        "Answer the question using only the numbered sources below, in the question's language. \
         Cite sources inline as [1], [2]. If they do not answer it, say what is missing. Keep it \
         to a short paragraph or a few bullets.\n\nQuestion: {query}\n\nSources:\n{}",
        serde_json::to_string_pretty(&sources).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_query_finds_libraries_knowledge_and_chats() {
        let dir = std::env::temp_dir().join(format!("lumo-searchall-{}", std::process::id()));
        let lib = super::super::libraries::create(&dir, "Company docs", "").unwrap();
        super::super::libraries::add_text(&dir, &lib.id, "handbook.md", "The on-call rotation changes every Monday.").unwrap();
        super::super::knowledge::write(&dir, None, "On-call", "Escalate to Marta during the on-call week.", "user", &[]).unwrap();
        let results = search(&dir, "on-call rotation", false).await;
        let kinds: Vec<&str> = results.cards.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"library"), "{kinds:?}");
        assert!(kinds.contains(&"knowledge"), "{kinds:?}");
        assert_eq!(results.cards[0].target, "Company docs:handbook.md");
        assert!(search(&dir, "   ", true).await.cards.is_empty());
        let prompt = answer_prompt("Who do I escalate to?", &results.cards);
        assert!(prompt.contains("\"n\": 1") && prompt.contains("[1]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn card_text_reads_cleanly() {
        let raw = "Summarize in two lines. <attachment name=\"Page: Acme\"> # Acme account\n- **Contract renewal:** 14 April </attachment>";
        assert_eq!(clip(raw, 200), "Summarize in two lines. Acme account Contract renewal: 14 April");
        assert_eq!(clip("word ".repeat(100).as_str(), 9), "word word…");
    }

}
