//! The interface in other languages (LibreChat's multilingual UI).
//!
//! `i18n/catalog.json` lists the interface's English strings (built from the page's source by
//! `scripts/ui-strings.mjs`). Portuguese, Spanish, French and German translations of the whole
//! catalog ship with the app (`i18n/<code>.json`, made with `scripts/translate_ui.py`). For the
//! other languages, or strings added since, the user's own model can translate what is missing;
//! that is kept per language in `chat/ui-translations/<code>.json`, so it is asked only once.
//!
//! Only catalog strings are ever sent: no messages, titles, names or anything the user typed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rig_core::completion::message::Message;
use rig_core::completion::CompletionRequest;

/// Languages the interface can be shown in, by code, with their own names.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("pt", "Português"),
    ("es", "Español"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("ja", "日本語"),
    ("zh", "中文"),
    ("ar", "العربية"),
    ("he", "עברית"),
];

/// Strings per request: enough to be quick, small enough for any model.
const BATCH: usize = 120;

const CATALOG: &str = include_str!("../../i18n/catalog.json");

/// The translations that ship with the app.
fn bundled(code: &str) -> Option<&'static str> {
    match code {
        "pt" => Some(include_str!("../../i18n/pt.json")),
        "es" => Some(include_str!("../../i18n/es.json")),
        "fr" => Some(include_str!("../../i18n/fr.json")),
        "de" => Some(include_str!("../../i18n/de.json")),
        "ar" => Some(include_str!("../../i18n/ar.json")),
        _ => None,
    }
}

pub fn catalog() -> Vec<String> {
    serde_json::from_str(CATALOG).unwrap_or_default()
}

/// How much of the catalog a language covers.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub known: usize,
    pub total: usize,
}

pub fn coverage(config_dir: &Path, code: &str) -> Coverage {
    let all = translations(config_dir, code);
    let catalog = catalog();
    Coverage {
        known: catalog.iter().filter(|s| all.contains_key(*s)).count(),
        total: catalog.len(),
    }
}

/// Everything known for `code`: the bundled translations, then the model's for the rest.
pub fn translations(config_dir: &Path, code: &str) -> BTreeMap<String, String> {
    let mut all = load(config_dir, code);
    if let Some(text) = bundled(code) {
        let shipped: BTreeMap<String, String> = serde_json::from_str(text).unwrap_or_default();
        all.extend(shipped);
    }
    all
}

fn path(config_dir: &Path, code: &str) -> Option<PathBuf> {
    LANGUAGES.iter().any(|(c, _)| *c == code).then(|| {
        config_dir
            .join("chat")
            .join("ui-translations")
            .join(format!("{code}.json"))
    })
}

/// What has been translated for `code` so far.
pub fn load(config_dir: &Path, code: &str) -> BTreeMap<String, String> {
    path(config_dir, code)
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(config_dir: &Path, code: &str, map: &BTreeMap<String, String>) -> Result<(), String> {
    let p = path(config_dir, code).ok_or("That language is not offered.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::chat::at_rest::write(
        p,
        serde_json::to_string_pretty(map).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// A translation keeps every `{n}` placeholder of the original, and no others.
fn placeholders(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let inner = &rest[start + 1..start + len];
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
            out.push(rest[start..=start + len].to_string());
        }
        rest = &rest[start + len + 1..];
    }
    out.sort();
    out
}

pub fn keeps_placeholders(original: &str, translated: &str) -> bool {
    placeholders(original) == placeholders(translated)
}

/// Reads the model's reply: a JSON object from each original to its translation. Entries that
/// are missing, empty or lose a placeholder are dropped.
pub fn parse_reply(reply: &str, asked: &[String]) -> BTreeMap<String, String> {
    let Some(start) = reply.find('{') else {
        return BTreeMap::new();
    };
    let Some(end) = reply.rfind('}') else {
        return BTreeMap::new();
    };
    let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&reply[start..=end]) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(k, v)| asked.contains(k) && !v.trim().is_empty() && keeps_placeholders(k, v))
        .collect()
}

/// Translates the catalog strings not yet known for `code` with `model`.
pub async fn translate(config_dir: &Path, model: &str, code: &str) -> Result<Coverage, String> {
    let (_, name) = LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .ok_or("That language is not offered.")?;
    let mut known = load(config_dir, code);
    let all = translations(config_dir, code);
    let mut wanted: Vec<String> = catalog()
        .into_iter()
        .filter(|s| !all.contains_key(s))
        .collect();
    wanted.sort();
    wanted.dedup();
    for batch in wanted.chunks(BATCH) {
        let list = serde_json::to_string_pretty(batch).map_err(|e| e.to_string())?;
        let prompt = format!(
            "Translate these user-interface strings of a desktop AI assistant app called Cortex from English into {name} ({code}). \
             Use the usual words of desktop software in that language, the same tone (short, plain, friendly), and keep \
             each string's capitalisation style, punctuation and length close to the original. Keep `{{0}}`-style \
             placeholders, product names (Cortex, Markdown, MCP, OpenAPI, GitHub, Python…), keyboard keys and code \
             exactly as they are. Reply with one JSON object mapping each original string to its translation, and \
             nothing else.\n\n{list}"
        );
        let request = CompletionRequest::new(Message::user(prompt)).max_tokens(16_000u64);
        let reply = crate::chat::engine::ask_text(config_dir, model, request, 180).await?;
        let got = parse_reply(&reply, batch);
        if got.is_empty() {
            return Err("The model's translation could not be read; try another model.".into());
        }
        known.extend(got);
        save(config_dir, code, &known)?;
    }
    Ok(coverage(config_dir, code))
}

/// Forgets the model's translations for `code` (the bundled ones stay).
pub fn reset(config_dir: &Path, code: &str) -> Result<(), String> {
    match path(config_dir, code) {
        Some(p) if p.exists() => std::fs::remove_file(p).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_must_survive() {
        assert!(keeps_placeholders(
            "{0} conversations, {1} MB",
            "{0} conversas, {1} MB"
        ));
        assert!(!keeps_placeholders("{0} conversations", "conversas"));
        assert!(!keeps_placeholders("Save", "Guardar {0}"));
        assert!(
            keeps_placeholders("Use {name}", "Usar {nome}"),
            "only numbered placeholders count"
        );
    }

    #[test]
    fn replies_are_read_strictly() {
        let asked = vec![
            "Save".to_string(),
            "{0} sent".to_string(),
            "Cancel".to_string(),
        ];
        let reply = "Here you go:\n```json\n{\"Save\": \"Guardar\", \"{0} sent\": \"enviados\", \"Cancel\": \"\", \"Other\": \"Outro\"}\n```";
        let got = parse_reply(reply, &asked);
        assert_eq!(got.len(), 1);
        assert_eq!(got["Save"], "Guardar");
        assert!(parse_reply("no json", &asked).is_empty());
    }

    #[test]
    fn the_bundled_languages_cover_the_catalog() {
        let catalog = catalog();
        assert!(catalog.len() > 1000);
        for code in ["pt", "es", "fr", "de", "ar"] {
            let map: BTreeMap<String, String> =
                serde_json::from_str(bundled(code).unwrap()).unwrap();
            let covered = catalog.iter().filter(|s| map.contains_key(*s)).count();
            assert!(
                covered * 100 >= catalog.len() * 97,
                "{code}: {covered} of {}",
                catalog.len()
            );
            for (en, tr) in &map {
                assert!(keeps_placeholders(en, tr), "{code}: {en} → {tr}");
            }
        }
    }

    #[test]
    fn only_offered_languages_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save(dir.path(), "../x", &BTreeMap::new()).is_err());
        let mut m = BTreeMap::new();
        m.insert("Save".to_string(), "Guardar".to_string());
        save(dir.path(), "pt", &m).unwrap();
        assert_eq!(load(dir.path(), "pt")["Save"], "Guardar");
        reset(dir.path(), "pt").unwrap();
        assert!(load(dir.path(), "pt").is_empty());
    }
}
