//! A reader for sources (Vibe shows AFP articles in its own reader): an answer's source opens in
//! the side panel as plain, readable text (title, site, author, date, picture, headings,
//! paragraphs, lists, quotes) without the page's menus, ads or scripts.
//!
//! The article is the page's `<article>` or `<main>` when it has one, else the block holding the
//! most paragraph text. Nothing of the page runs; the text is shown as text.

use scraper::{ElementRef, Html, Selector};
use serde::Serialize;

const MAX_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// `h` (a heading), `p`, `li`, `quote` or `code`.
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub url: String,
    pub title: String,
    pub site: String,
    pub byline: String,
    pub published: String,
    pub image: String,
    pub blocks: Vec<Block>,
    pub words: usize,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("a valid selector")
}

fn meta(doc: &Html, names: &[&str]) -> String {
    for name in names {
        for attr in ["property", "name", "itemprop"] {
            if let Some(content) = doc
                .select(&sel(&format!("meta[{attr}=\"{name}\"]")))
                .next()
                .and_then(|m| m.value().attr("content"))
            {
                let content = content.trim();
                if !content.is_empty() {
                    return content.to_string();
                }
            }
        }
    }
    String::new()
}

fn text_of(el: ElementRef<'_>) -> String {
    el.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parts of a page that are never the article.
fn is_chrome(el: ElementRef<'_>) -> bool {
    let v = el.value();
    if matches!(
        v.name(),
        "nav"
            | "aside"
            | "footer"
            | "header"
            | "form"
            | "script"
            | "style"
            | "noscript"
            | "svg"
            | "button"
            | "figcaption"
            | "iframe"
    ) {
        return true;
    }
    let marks = format!(
        "{} {}",
        v.attr("class").unwrap_or_default(),
        v.attr("id").unwrap_or_default()
    )
    .to_lowercase();
    [
        "comment",
        "share",
        "social",
        "related",
        "newsletter",
        "advert",
        "promo",
        "cookie",
        "subscribe",
        "sidebar",
        "breadcrumb",
    ]
    .iter()
    .any(|m| marks.contains(m))
        || v.attr("role")
            .is_some_and(|r| matches!(r, "navigation" | "banner" | "contentinfo" | "complementary"))
        || v.attr("aria-hidden") == Some("true")
}

fn inside_chrome(el: ElementRef<'_>, root: ElementRef<'_>) -> bool {
    el.ancestors()
        .take_while(|a| a.id() != root.id())
        .filter_map(ElementRef::wrap)
        .any(is_chrome)
}

/// Paragraph text inside `el`, for picking the article's block.
fn paragraph_weight(el: ElementRef<'_>) -> usize {
    el.select(&sel("p"))
        .filter(|p| !inside_chrome(*p, el))
        .map(|p| text_of(p).len())
        .filter(|n| *n > 40)
        .sum()
}

fn content_root(doc: &Html) -> Option<ElementRef<'_>> {
    for s in ["article", "main", "[role=main]", "[itemprop=articleBody]"] {
        if let Some(el) = doc.select(&sel(s)).max_by_key(|e| paragraph_weight(*e)) {
            if paragraph_weight(el) > 200 {
                return Some(el);
            }
        }
    }
    doc.select(&sel("div, section")).max_by_key(|e| {
        // The innermost block with the text, not the whole page: weight minus what its
        // children of the same kind already hold.
        let w = paragraph_weight(*e);
        let child_best = e
            .children()
            .filter_map(ElementRef::wrap)
            .map(paragraph_weight)
            .max()
            .unwrap_or(0);
        w.saturating_sub(child_best / 2)
    })
}

/// Reads an article out of a page's HTML.
pub(in crate::chat::grounding) fn extract(html: &str, url: &str) -> Article {
    let doc = Html::parse_document(html);
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default();
    let mut title = meta(&doc, &["og:title", "twitter:title"]);
    if title.is_empty() {
        title = doc
            .select(&sel("h1"))
            .next()
            .map(text_of)
            .or_else(|| doc.select(&sel("title")).next().map(text_of))
            .unwrap_or_default();
    }
    let site = Some(meta(&doc, &["og:site_name", "application-name"]))
        .filter(|s| !s.is_empty())
        .unwrap_or(host);
    let byline = meta(&doc, &["author", "article:author", "byl", "parsely-author"]);
    let published = meta(
        &doc,
        &[
            "article:published_time",
            "datePublished",
            "pubdate",
            "date",
            "dc.date",
        ],
    );
    let mut image = meta(&doc, &["og:image", "twitter:image"]);
    if let (Ok(base), false) = (url::Url::parse(url), image.is_empty()) {
        image = base.join(&image).map(|u| u.to_string()).unwrap_or_default();
    }
    if !image.starts_with("https://") {
        image.clear();
    }
    let mut blocks: Vec<Block> = Vec::new();
    if let Some(root) = content_root(&doc) {
        for el in root.select(&sel("h1, h2, h3, h4, p, li, blockquote, pre")) {
            if inside_chrome(el, root) || is_chrome(el) {
                continue;
            }
            // A list item or quote made of paragraphs is read through those paragraphs.
            let name = el.value().name();
            if matches!(name, "li" | "blockquote") && el.select(&sel("p")).next().is_some() {
                continue;
            }
            if name == "p"
                && el
                    .ancestors()
                    .filter_map(ElementRef::wrap)
                    .any(|a| a.value().name() == "pre")
            {
                continue;
            }
            let text = if name == "pre" {
                el.text().collect::<String>().trim_end().to_string()
            } else {
                text_of(el)
            };
            if text.is_empty() || (name != "pre" && text == title && blocks.is_empty()) {
                continue;
            }
            let kind = match name {
                "h1" | "h2" | "h3" | "h4" => "h",
                "li" => "li",
                "blockquote" => "quote",
                "pre" => "code",
                _ => "p",
            };
            // Short lines that are not headings are usually leftovers (buttons, captions).
            if kind == "p"
                && text.split_whitespace().count() < 4
                && !text.ends_with(['.', '!', '?', ':'])
            {
                continue;
            }
            if blocks.last().is_some_and(|b| b.text == text) {
                continue;
            }
            blocks.push(Block {
                kind: kind.into(),
                text,
            });
        }
    }
    let words = blocks
        .iter()
        .map(|b| b.text.split_whitespace().count())
        .sum();
    Article {
        url: url.into(),
        title,
        site,
        byline,
        published,
        image,
        blocks,
        words,
    }
}

/// Fetches `url` and reads its article.
pub async fn read(url: &str) -> Result<Article, String> {
    let parsed = url::Url::parse(url).map_err(|_| "That is not a web address.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only web pages can be read here.".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Cortex reader",
        )
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(parsed.clone()).send().await.map_err(|e| {
        format!(
            "The page could not be loaded: {}",
            cortex_kernel::context::short_error(&e.to_string())
        )
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "The site would not send the page here (HTTP {}); open it in the browser.",
            response.status().as_u16()
        ));
    }
    let final_url = response.url().to_string();
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("The page is too large to read here; open it in the browser.".into());
    }
    let article = extract(&String::from_utf8_lossy(&bytes), &final_url);
    if article.words < 30 {
        return Err("No article was found on this page; open it in the browser.".into());
    }
    Ok(article)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html><head><title>Rail strike ends | Daily News</title>
        <meta property="og:title" content="Rail strike ends after three days">
        <meta property="og:site_name" content="Daily News"><meta name="author" content="Ana Silva">
        <meta property="article:published_time" content="2026-11-17T08:00:00Z"><meta property="og:image" content="/img/train.jpg">
        </head><body>
        <header><nav><a href="/">Home</a><a href="/world">World</a></nav></header>
        <div class="layout"><div class="content">
          <article>
            <h1>Rail strike ends after three days</h1>
            <p>Trains ran again on Tuesday morning after unions and the operator reached an agreement late on Monday night.</p>
            <div class="share-buttons"><p>Share this on social media with your friends and family today.</p></div>
            <h2>What was agreed</h2>
            <ul><li>A 4% raise this year</li><li>Shorter night shifts</li></ul>
            <blockquote><p>“This is a fair deal for everyone who keeps the trains running,” the union said.</p></blockquote>
            <p>Commuters faced delays of up to two hours during the strike, which affected every line in the region.</p>
            <p>Share</p>
          </article>
          <aside class="related"><p>Related: ten other stories you might like to read this week.</p></aside>
        </div></div>
        <footer><p>Copyright Daily News, all rights reserved, every year since forever.</p></footer>
        <script>alert(1)</script></body></html>"#;

    #[test]
    fn the_article_is_read_without_the_page_around_it() {
        let a = extract(PAGE, "https://www.dailynews.example/rail");
        assert_eq!(a.title, "Rail strike ends after three days");
        assert_eq!(
            (a.site.as_str(), a.byline.as_str()),
            ("Daily News", "Ana Silva")
        );
        assert_eq!(a.published, "2026-11-17T08:00:00Z");
        assert_eq!(a.image, "https://www.dailynews.example/img/train.jpg");
        let kinds: Vec<(&str, &str)> = a
            .blocks
            .iter()
            .map(|b| (b.kind.as_str(), b.text.as_str()))
            .collect();
        assert_eq!(kinds[0].0, "p", "the repeated title is not a block");
        assert!(kinds.contains(&("h", "What was agreed")));
        assert!(kinds.contains(&("li", "A 4% raise this year")));
        assert!(
            kinds
                .iter()
                .any(|(k, t)| *k == "p" && t.starts_with("“This is a fair deal")),
            "a quote's paragraph is kept"
        );
        let all = a
            .blocks
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        for gone in ["Home", "Share this", "Related:", "Copyright", "alert"] {
            assert!(!all.contains(gone), "{gone} should be left out");
        }
        assert_eq!(a.blocks.last().unwrap().text, "Commuters faced delays of up to two hours during the strike, which affected every line in the region.");
    }

    #[test]
    fn pages_without_an_article_element_use_their_densest_block() {
        let html = r#"<html><body><div id="menu"><p>Menu item one</p></div><div id="story"><p>The council approved the new park on the riverside after a long debate on Wednesday evening.</p><p>Work starts in spring and should take about eighteen months, the mayor said at the meeting.</p></div></body></html>"#;
        let a = extract(html, "http://town.example/park");
        assert_eq!(a.blocks.len(), 2);
        assert_eq!(a.site, "town.example");
        assert!(a.image.is_empty());
    }
}
