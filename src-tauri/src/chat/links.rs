//! Links into the app (`proton-lumo://` or `cortex://`), as Open WebUI's `?q=` and LobeHub's share
//! links allow: launchers, scripts, browser bookmarks and other apps can start a chat.
//!
//! - `cortex://new?prompt=…&model=vendor/model&mode=work|code`: a new chat with the prompt typed in
//!   the composer. It is **never sent by the link**: another program could otherwise make the
//!   assistant act (search, run tools) without the user seeing the request first.
//! - `cortex://chat/<conversation id>`: opens that conversation.
//!
//! A link arriving before the window is ready waits in [`PendingLink`] until the page asks.

use std::sync::Mutex;

use serde::Serialize;

/// Characters of a prompt taken from a link.
const MAX_PROMPT: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Link {
    #[serde(rename_all = "camelCase")]
    New {
        prompt: String,
        model: Option<String>,
        mode: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Open { conversation_id: String },
}

/// The app link a URL names, if it is one.
pub fn parse(url: &url::Url) -> Option<Link> {
    if !matches!(url.scheme(), "proton-lumo" | "cortex") {
        return None;
    }
    let query = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    match url.host_str()? {
        "new" => {
            let prompt: String = query("prompt")
                .or_else(|| query("q"))
                .unwrap_or_default()
                .chars()
                .take(MAX_PROMPT)
                .collect();
            let model = query("model").filter(|m| m.contains('/') && m.len() <= 200);
            let mode = query("mode").filter(|m| m == "work" || m == "code");
            Some(Link::New {
                prompt,
                model,
                mode,
            })
        }
        "chat" => {
            let id = url.path().trim_matches('/');
            if !super::store::valid_id(id) {
                return None;
            }
            Some(Link::Open {
                conversation_id: id.to_string(),
            })
        }
        _ => None,
    }
}

/// A link that arrived before the page could take it.
#[derive(Default)]
pub struct PendingLink(Mutex<Option<Link>>);

impl PendingLink {
    pub fn put(&self, link: Link) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(link);
    }

    pub fn take(&self) -> Option<Link> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(s: &str) -> Option<Link> {
        parse(&url::Url::parse(s).unwrap())
    }

    #[test]
    fn links_start_or_open_chats() {
        assert_eq!(
            link("cortex://new?prompt=Plan%20a%20trip%20to%20Porto&model=xai/grok-4&mode=code"),
            Some(Link::New {
                prompt: "Plan a trip to Porto".into(),
                model: Some("xai/grok-4".into()),
                mode: Some("code".into())
            })
        );
        assert_eq!(
            link("proton-lumo://new?q=hello&mode=admin&model=nope"),
            Some(Link::New {
                prompt: "hello".into(),
                model: None,
                mode: None
            })
        );
        assert_eq!(
            link("cortex://new"),
            Some(Link::New {
                prompt: String::new(),
                model: None,
                mode: None
            })
        );
        let long = format!("cortex://new?prompt={}", "a".repeat(20_000));
        assert!(
            matches!(link(&long), Some(Link::New { prompt, .. }) if prompt.len() == MAX_PROMPT)
        );
        assert_eq!(
            link("cortex://chat/abc-123"),
            Some(Link::Open {
                conversation_id: "abc-123".into()
            })
        );
        assert_eq!(
            link("cortex://chat/..%2F..%2Fetc"),
            None,
            "only plain conversation ids"
        );
        assert_eq!(
            link("proton-lumo://oauth/callback?code=x"),
            None,
            "sign-in callbacks are not app links"
        );
        assert_eq!(link("https://new?prompt=x"), None);
        let pending = PendingLink::default();
        pending.put(Link::Open {
            conversation_id: "a".into(),
        });
        assert!(pending.take().is_some() && pending.take().is_none());
    }
}
