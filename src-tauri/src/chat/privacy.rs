//! The privacy filter: personal data is swapped for placeholders before a request leaves for a
//! cloud provider, and swapped back in the reply.
//!
//! Emails, phone numbers, IBANs and card numbers in what the user typed are replaced with
//! `[EMAIL_1]`-style placeholders in the request only; the stored conversation keeps the real
//! text, and the model's reply is restored before the user sees it. Local providers (Ollama,
//! llama.cpp) never leave the machine, so nothing is filtered for them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use rig_core::completion::message::{AssistantContent, Message, UserContent};
use serde::{Deserialize, Serialize};

use super::store::{chat_dir, write_json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct PrivacySettings {
    pub enabled: bool,
}

fn path(config_dir: &Path) -> PathBuf {
    chat_dir(config_dir).join("privacy.json")
}

pub fn load(config_dir: &Path) -> PrivacySettings {
    crate::chat::at_rest::read_to_string(path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, settings: &PrivacySettings) -> Result<(), String> {
    write_json(&path(config_dir), settings)
}

/// Whether requests to `vendor` leave this machine.
pub fn is_cloud(vendor: &str) -> bool {
    !matches!(vendor, "ollama" | "llamacpp")
}

fn patterns() -> &'static [(&'static str, Regex)] {
    static PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                "EMAIL",
                Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("email"),
            ),
            (
                "IBAN",
                Regex::new(r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){3,7}(?: ?[A-Z0-9]{1,4})?\b")
                    .expect("iban"),
            ),
            ("CARD", Regex::new(r"\b(?:\d[ -]?){13,19}\b").expect("card")),
            (
                "PHONE",
                Regex::new(r"(?:\+\d{1,3}[ .-]?)?(?:\(?\d{2,4}\)?[ .-]?){2,4}\d{3,4}\b")
                    .expect("phone"),
            ),
        ]
    })
}

/// The kinds of sensitive data in `text` that do not belong in saved knowledge: card numbers,
/// bank accounts, and secrets (passwords, PINs, API keys and tokens).
pub fn sensitive_kinds(text: &str) -> Vec<&'static str> {
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let secret = SECRET.get_or_init(|| {
        Regex::new(r"(?i)\b(password|passwd|passcode|pin|api[ _-]?key|secret|token|senha|palavra-passe)\b\s*(is|é|:|=)\s*\S{3,}|\b(sk|pk|rk)-[A-Za-z0-9_-]{16,}|\bgh[pousr]_[A-Za-z0-9]{20,}|\bAKIA[0-9A-Z]{16}\b|\bxox[abp]-[A-Za-z0-9-]{10,}").expect("secret")
    });
    let mut out = Vec::new();
    for (kind, re) in patterns() {
        let hit = re.find_iter(text).any(|m| match *kind {
            "CARD" => m.as_str().chars().filter(char::is_ascii_digit).count() >= 13,
            "IBAN" => true,
            _ => false,
        });
        if hit {
            out.push(if *kind == "CARD" {
                "a card number"
            } else {
                "a bank account number"
            });
        }
    }
    // An IBAN's digits also read as a card number; it is one thing.
    if out.contains(&"a bank account number") {
        out.retain(|k| *k != "a card number");
    }
    if secret.is_match(text) {
        out.push("a password, key or token");
    }
    out
}

/// The placeholders of one request, so the reply can be restored.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Redactions {
    /// (placeholder, original), in the order they were made.
    pub pairs: Vec<(String, String)>,
}

impl Redactions {
    fn placeholder(&mut self, kind: &str, original: &str) -> String {
        if let Some((p, _)) = self.pairs.iter().find(|(_, o)| o == original) {
            return p.clone();
        }
        let n = self
            .pairs
            .iter()
            .filter(|(p, _)| p.starts_with(&format!("[{kind}_")))
            .count()
            + 1;
        let p = format!("[{kind}_{n}]");
        self.pairs.push((p.clone(), original.to_string()));
        p
    }

    /// Replaces personal data in `text`, remembering each placeholder.
    pub fn redact(&mut self, text: &str) -> String {
        let mut out = text.to_string();
        for (kind, re) in patterns() {
            let found: Vec<String> = re
                .find_iter(&out)
                .map(|m| m.as_str().to_string())
                // A phone or card needs enough digits to be one; years and prices are not.
                .filter(|m| {
                    !matches!(*kind, "PHONE" | "CARD")
                        || m.chars().filter(char::is_ascii_digit).count() >= 9
                })
                .collect();
            for original in found {
                let p = self.placeholder(kind, original.trim());
                out = out.replace(original.trim(), &p);
            }
        }
        out
    }

    /// Puts the originals back.
    pub fn restore(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (p, original) in &self.pairs {
            out = out.replace(p, original);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Redacts the text the user typed in `messages` (not tool results, which come from the user's
/// own machine and services and are needed verbatim).
pub fn redact_messages(messages: &[Message], redactions: &mut Redactions) -> Vec<Message> {
    messages
        .iter()
        .map(|m| match m {
            Message::User { content } => Message::User {
                content: content
                    .iter()
                    .map(|c| match c {
                        UserContent::Text(t) => UserContent::text(redactions.redact(&t.text)),
                        other => other.clone(),
                    })
                    .collect(),
            },
            Message::Assistant(a) => {
                let mut a = a.clone();
                for c in a.content.iter_mut() {
                    if let AssistantContent::Text(t) = c {
                        t.text = redactions.redact(&t.text);
                    }
                }
                Message::Assistant(a)
            }
            other => other.clone(),
        })
        .collect()
}

/// Restores placeholders in a reply before it is stored and shown.
pub fn restore_message(message: Message, redactions: &Redactions) -> Message {
    if redactions.is_empty() {
        return message;
    }
    match message {
        Message::Assistant(mut a) => {
            for c in a.content.iter_mut() {
                match c {
                    AssistantContent::Text(t) => t.text = redactions.restore(&t.text),
                    AssistantContent::ToolCall(call) => {
                        for value in call.function.arguments.values_mut() {
                            if let serde_json::Value::String(s) = value {
                                *s = redactions.restore(s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Message::Assistant(a)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_data_is_replaced_and_restored() {
        let mut r = Redactions::default();
        let text = "Email ana.silva@example.pt or call +351 912 345 678. IBAN PT50 0002 0123 1234 5678 9015 4. Card 4111 1111 1111 1111. Budget 2026: 900 EUR.";
        let redacted = r.redact(text);
        assert!(!redacted.contains("ana.silva@example.pt"), "{redacted}");
        assert!(redacted.contains("[EMAIL_1]"));
        assert!(redacted.contains("[IBAN_1]"), "{redacted}");
        assert!(redacted.contains("[CARD_1]"), "{redacted}");
        assert!(redacted.contains("[PHONE_1]"), "{redacted}");
        assert!(
            redacted.contains("2026") && redacted.contains("900 EUR"),
            "{redacted}"
        );
        assert_eq!(r.restore(&redacted), text);
    }

    #[test]
    fn the_same_value_keeps_its_placeholder() {
        let mut r = Redactions::default();
        let a = r.redact("write to bob@x.org");
        let b = r.redact("bob@x.org again, and amy@y.org");
        assert_eq!(a, "write to [EMAIL_1]");
        assert_eq!(b, "[EMAIL_1] again, and [EMAIL_2]");
    }

    #[test]
    fn messages_are_redacted_and_replies_restored() {
        let mut r = Redactions::default();
        let out = redact_messages(&[Message::user("my mail is jo@z.com")], &mut r);
        assert_eq!(out, vec![Message::user("my mail is [EMAIL_1]")]);
        let reply = restore_message(Message::assistant("I'll write to [EMAIL_1]."), &r);
        assert_eq!(reply, Message::assistant("I'll write to jo@z.com."));
        assert!(!is_cloud("ollama") && is_cloud("openai"));
    }

    #[test]
    fn sensitive_data_is_recognised() {
        assert_eq!(
            sensitive_kinds("My card is 4111 1111 1111 1111"),
            ["a card number"]
        );
        assert_eq!(
            sensitive_kinds("Rent goes to PT50 0002 0123 1234 5678 9015 4"),
            ["a bank account number"]
        );
        assert_eq!(
            sensitive_kinds("the wifi password is hunter22"),
            ["a password, key or token"]
        );
        assert_eq!(
            sensitive_kinds("use key sk-abcdefghijklmnop1234 for the API"),
            ["a password, key or token"]
        );
        assert!(
            sensitive_kinds("Rita's email is rita@acme.example, phone +351 912 345 678").is_empty()
        );
        assert!(sensitive_kinds("Prefers metric units; born in 1988").is_empty());
    }
}
