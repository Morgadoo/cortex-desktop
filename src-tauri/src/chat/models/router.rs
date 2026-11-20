//! The "Auto" model (Onyx's auto model router): each message goes to a fast model or a strong
//! one, chosen from the message itself, so quick questions stay quick and cheap and demanding
//! ones get the stronger model. The two models are set under Providers.

use crate::chat::providers::{self, ModelView};

/// The reference the model picker uses for Auto.
pub const AUTO: &str = "auto/router";

/// Which model a message goes to, and why, in a few words.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub strong: bool,
    pub reason: &'static str,
}

/// Words that mark work worth the stronger model, in a few languages.
const DEMANDING: &[&str] = &[
    "analy",
    "analis",
    "anális",
    "análise",
    "compare",
    "compar",
    "plan ",
    "design",
    "architect",
    "debug",
    "refactor",
    "prove",
    "proof",
    "derive",
    "step by step",
    "in detail",
    "thorough",
    "strategy",
    "estratég",
    "evaluate",
    "trade-off",
    "tradeoff",
    "review",
    "optimi",
    "write a report",
    "write an essay",
    "business plan",
    "research",
    "investig",
    "explain why",
    "porquê",
    "por qué",
];

/// Picks the model for a message: the strong one for code, long or attached input, images,
/// several questions at once, and words that ask for analysis or design; the fast one otherwise.
pub fn route(text: &str, images: usize) -> Route {
    let lower = text.to_lowercase();
    if text.contains("```") || text.contains("<attachment") {
        return Route {
            strong: true,
            reason: if text.contains("```") {
                "code in the message"
            } else {
                "attached content"
            },
        };
    }
    if images > 0 {
        return Route {
            strong: true,
            reason: "an image to look at",
        };
    }
    if text.chars().count() > 1_200 {
        return Route {
            strong: true,
            reason: "a long message",
        };
    }
    if text.matches('?').count() >= 3 {
        return Route {
            strong: true,
            reason: "several questions at once",
        };
    }
    let code_words = [
        "fn ",
        "def ",
        "class ",
        "function ",
        "select ",
        "import ",
        "=>",
        "();",
        "#include",
    ];
    if code_words.iter().any(|w| lower.contains(w)) {
        return Route {
            strong: true,
            reason: "code in the message",
        };
    }
    if DEMANDING.iter().any(|w| lower.contains(w)) {
        return Route {
            strong: true,
            reason: "it asks for analysis or design",
        };
    }
    Route {
        strong: false,
        reason: "a quick message",
    }
}

/// The fast and strong models, when both are set.
pub fn models(file: &providers::ProvidersFile) -> Option<(String, String)> {
    Some((
        file.router_fast.clone().filter(|m| !m.is_empty())?,
        file.router_strong.clone().filter(|m| !m.is_empty())?,
    ))
}

/// The Auto entry for the model picker, when it is set up.
pub(in crate::chat::models) fn model_view(file: &providers::ProvidersFile) -> Option<ModelView> {
    models(file).map(|_| ModelView {
        reference: AUTO.into(),
        vendor: "auto".into(),
        provider_label: "Auto".into(),
        id: "router".into(),
        name: "Auto".into(),
        reasoning: false,
        tools: true,
        context_window: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_messages_go_fast_and_demanding_ones_strong() {
        assert!(!route("What's the capital of Portugal?", 0).strong);
        assert!(!route("Thanks! And tomorrow's date?", 0).strong);
        assert_eq!(
            route("Fix this:\n```rust\nfn main() {}\n```", 0).reason,
            "code in the message"
        );
        assert!(route("Why does `def foo(x):` fail in Python?", 0).strong);
        assert_eq!(
            route("What is in this picture?", 1).reason,
            "an image to look at"
        );
        assert_eq!(
            route("Compare Postgres and SQLite for a desktop app", 0).reason,
            "it asks for analysis or design"
        );
        assert_eq!(
            route("Summarize it\n<attachment name=\"a.txt\">x</attachment>", 0).reason,
            "attached content"
        );
        assert_eq!(route(&"word ".repeat(300), 0).reason, "a long message");
        assert_eq!(
            route("Who? What? When?", 0).reason,
            "several questions at once"
        );
        assert!(route("Faz uma análise do mercado", 0).strong);
    }
}
