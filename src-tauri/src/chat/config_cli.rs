//! `lumo-desktop config …` (Vibe's `vibe config schema`): the settings files described as JSON
//! Schema, and a check that the ones on this computer can be read.
//!
//! ```text
//! lumo-desktop config schema              every settings file, one schema
//! lumo-desktop config schema providers    one file's schema
//! lumo-desktop config check               reads each file and says which cannot be used
//! lumo-desktop config path                the settings folder
//! ```
//!
//! The schemas come from the same types the app reads, so they cannot drift. Editors that take
//! JSON Schema (VS Code's `json.schemas`) can then check and complete hand edits.

use std::path::Path;

use schemars::{schema_for, Schema};
use serde_json::{json, Value};

/// A settings file: its name for the command line, where it lives, its schema and a reader.
struct File {
    name: &'static str,
    path: &'static str,
    about: &'static str,
    schema: fn() -> Schema,
    check: fn(&str) -> Result<(), String>,
}

fn read<T: serde::de::DeserializeOwned>(text: &str) -> Result<(), String> {
    serde_json::from_str::<T>(text).map(|_| ()).map_err(|e| e.to_string())
}

const FILES: &[File] = &[
    File { name: "providers", path: "chat/providers.json", about: "Model providers, the default, backup, Auto and media models. Keys are in the keychain.", schema: || schema_for!(super::providers::ProvidersFile), check: read::<super::providers::ProvidersFile> },
    File { name: "agents", path: "chat/agents.json", about: "Agents: instructions, model, tools, skills, libraries, vault secrets.", schema: || schema_for!(Vec<super::agents::Agent>), check: read::<Vec<super::agents::Agent>> },
    File { name: "tasks", path: "chat/tasks.json", about: "Scheduled tasks and their triggers.", schema: || schema_for!(Vec<super::tasks::Task>), check: read::<Vec<super::tasks::Task>> },
    File { name: "hooks", path: "chat/hooks.json", about: "Commands run before and after tool calls and when a reply finishes.", schema: || schema_for!(Vec<super::hooks::Hook>), check: read::<Vec<super::hooks::Hook>> },
    File { name: "audio", path: "chat/audio.json", about: "Speech to text and text to speech.", schema: || schema_for!(super::audio::AudioSettings), check: read::<super::audio::AudioSettings> },
    File { name: "memories", path: "chat/memories.json", about: "Facts the assistant knows, and whether it uses and saves them.", schema: || schema_for!(super::memory::Memories), check: read::<super::memory::Memories> },
    File { name: "vault", path: "chat/vault.json", about: "Names of vault secrets and who may use them. Values are in the keychain.", schema: || schema_for!(Vec<super::vault::Secret>), check: read::<Vec<super::vault::Secret>> },
    File { name: "sync", path: "chat/sync.json", about: "Sync between devices on this computer. The passphrase is in the keychain.", schema: || schema_for!(super::sync::Settings), check: read::<super::sync::Settings> },
    File { name: "api-server", path: "chat/api-server.json", about: "The local API.", schema: || schema_for!(super::api_server::ApiServerSettings), check: read::<super::api_server::ApiServerSettings> },
    File { name: "computer", path: "chat/computer.json", about: "Computer use.", schema: || schema_for!(super::computer::ComputerSettings), check: read::<super::computer::ComputerSettings> },
    File { name: "privacy", path: "chat/privacy.json", about: "The personal-data filter.", schema: || schema_for!(super::privacy::PrivacySettings), check: read::<super::privacy::PrivacySettings> },
    File { name: "dictation", path: "chat/dictation.json", about: "Dictation anywhere: shortcut and how text goes in.", schema: || schema_for!(crate::dictation::Prefs), check: read::<crate::dictation::Prefs> },
    File { name: "browser", path: "chat/browser.json", about: "Whether links open in Lumo's own browser.", schema: || schema_for!(crate::inapp_browser::Prefs), check: read::<crate::inapp_browser::Prefs> },
];

/// One schema describing every settings file, keyed by its path in the settings folder.
pub fn all_schemas() -> Value {
    let mut properties = serde_json::Map::new();
    for f in FILES {
        let mut schema = serde_json::to_value((f.schema)()).unwrap_or(Value::Null);
        if let Some(o) = schema.as_object_mut() {
            o.remove("$schema");
            o.insert("description".into(), json!(f.about));
        }
        properties.insert(f.path.into(), schema);
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Lumo settings",
        "description": "The JSON files in Lumo's settings folder (`lumo-desktop config path`), each under its path.",
        "type": "object",
        "properties": properties,
    })
}

/// Each file's state on this computer: `ok`, `missing (defaults apply)` or why it cannot be read.
pub fn check(config_dir: &Path) -> Vec<(&'static str, Result<&'static str, String>)> {
    FILES
        .iter()
        .map(|f| {
            let state = match std::fs::read_to_string(config_dir.join(f.path)) {
                Err(_) => Ok("missing (defaults apply)"),
                Ok(text) => (f.check)(&text).map(|_| "ok"),
            };
            (f.path, state)
        })
        .collect()
}

/// Edit distance, for "did you mean".
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Follows `{"$ref": "#/$defs/Name"}` within `root`.
fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")) {
        Some(name) => root.get("$defs").and_then(|d| d.get(name)).unwrap_or(schema),
        None => schema,
    }
}

/// Settings in `value` that `schema` does not know (a typo, or one an older or newer version
/// used), with the closest known name when one is near.
fn unknown_settings(value: &Value, schema: &Value, root: &Value, at: &str, out: &mut Vec<String>) {
    let schema = resolve(schema, root);
    match value {
        Value::Object(map) => {
            let Some(props) = schema.get("properties").and_then(Value::as_object) else {
                // Maps keyed by user names (providers by vendor) check their values.
                if let Some(values) = schema.get("additionalProperties").filter(|v| v.is_object()) {
                    for (k, v) in map {
                        unknown_settings(v, values, root, &format!("{at}{k}."), out);
                    }
                }
                return;
            };
            for (key, v) in map {
                match props.get(key) {
                    Some(s) => unknown_settings(v, s, root, &format!("{at}{key}."), out),
                    None => {
                        let near = props.keys().map(|k| (distance(&key.to_lowercase(), &k.to_lowercase()), k)).filter(|(d, _)| *d <= 3).min();
                        out.push(match near {
                            Some((_, k)) => format!("unknown setting `{at}{key}` (did you mean `{k}`?)"),
                            None => format!("unknown setting `{at}{key}`"),
                        });
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(item) = schema.get("items") {
                for (i, v) in items.iter().enumerate() {
                    unknown_settings(v, item, root, &format!("{at}[{i}]."), out);
                }
            }
        }
        _ => {}
    }
}

/// Unknown settings across the settings files, as `file: message`.
pub fn warnings(config_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in FILES {
        let Ok(text) = std::fs::read_to_string(config_dir.join(f.path)) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
        let schema = serde_json::to_value((f.schema)()).unwrap_or(Value::Null);
        let mut found = Vec::new();
        unknown_settings(&value, &schema, &schema, "", &mut found);
        out.extend(found.into_iter().map(|w| format!("{}: {}", f.path, w.replace(".`", "`"))));
    }
    out
}

/// At start: unknown settings are logged; with `LUMO_STRICT_CONFIG=1` (Vibe's `strict_config`)
/// the app refuses to start on them, or on a file it cannot read.
pub fn startup_check() -> Option<i32> {
    let dir = crate::reporting::early_config_dir()?;
    let problems: Vec<String> = check(&dir).into_iter().filter_map(|(p, s)| s.err().map(|e| format!("{p}: cannot be read: {e}"))).chain(warnings(&dir)).collect();
    for p in &problems {
        log::warn!("settings: {p}");
    }
    let strict = std::env::var("LUMO_STRICT_CONFIG").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if strict && !problems.is_empty() {
        eprintln!("lumo: LUMO_STRICT_CONFIG is set and the settings have problems:");
        for p in &problems {
            eprintln!("  {p}");
        }
        return Some(1);
    }
    None
}

/// Runs `config …` when asked; `None` to start the app.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("config") {
        return None;
    }
    Some(run(&args[1..], &mut std::io::stdout()))
}

pub fn run(args: &[String], out: &mut impl std::io::Write) -> i32 {
    let config_dir = crate::reporting::early_config_dir();
    match args.first().map(String::as_str) {
        Some("schema") => match args.get(1) {
            None => {
                let _ = writeln!(out, "{}", serde_json::to_string_pretty(&all_schemas()).unwrap_or_default());
                0
            }
            Some(name) => match FILES.iter().find(|f| f.name == name || f.path == name) {
                Some(f) => {
                    let _ = writeln!(out, "{}", serde_json::to_string_pretty(&(f.schema)()).unwrap_or_default());
                    0
                }
                None => {
                    eprintln!("lumo: no settings file called {name}; one of: {}", FILES.iter().map(|f| f.name).collect::<Vec<_>>().join(", "));
                    2
                }
            },
        },
        Some("check") => {
            let Some(dir) = config_dir else {
                eprintln!("lumo: the settings folder could not be found");
                return 1;
            };
            let mut bad = 0;
            let warned = warnings(&dir);
            for (path, state) in check(&dir) {
                match state {
                    Ok(s) => {
                        let _ = writeln!(out, "{path}: {s}");
                    }
                    Err(e) => {
                        bad += 1;
                        let _ = writeln!(out, "{path}: cannot be read: {e}");
                    }
                }
            }
            for w in &warned {
                let _ = writeln!(out, "warning: {w}");
            }
            i32::from(bad > 0)
        }
        Some("path") => match config_dir {
            Some(dir) => {
                let _ = writeln!(out, "{}", dir.display());
                0
            }
            None => 1,
        },
        _ => {
            eprintln!("usage: lumo-desktop config schema [file] | config check | config path\nfiles: {}", FILES.iter().map(|f| f.name).collect::<Vec<_>>().join(", "));
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_has_a_schema_that_matches_what_the_app_writes() {
        let all = all_schemas();
        let props = all["properties"].as_object().unwrap();
        assert_eq!(props.len(), FILES.len());
        // Field names are as saved (camelCase), with their descriptions from the code.
        let providers = &props["chat/providers.json"]["properties"];
        assert!(providers.get("defaultModel").is_some() && providers.get("default_model").is_none());
        let tasks = serde_json::to_string(&props["chat/tasks.json"]).unwrap();
        assert!(tasks.contains("newFeedItems") && tasks.contains("newFiles"));
        // What the app writes passes its own check.
        let dir = tempfile::tempdir().unwrap();
        super::super::providers::save(dir.path(), &super::super::providers::ProvidersFile { default_model: Some("x/y".into()), ..Default::default() }).unwrap();
        std::fs::write(dir.path().join("chat/hooks.json"), "{\"not\": \"a list\"}").unwrap();
        let states = check(dir.path());
        assert_eq!(states.iter().find(|(p, _)| *p == "chat/providers.json").unwrap().1, Ok("ok"));
        assert!(states.iter().find(|(p, _)| *p == "chat/hooks.json").unwrap().1.is_err());
        assert_eq!(states.iter().find(|(p, _)| *p == "chat/tasks.json").unwrap().1, Ok("missing (defaults apply)"));
        let mut out = Vec::new();
        assert_eq!(run(&["schema".into(), "vault".into()], &mut out), 0);
        assert!(String::from_utf8(out).unwrap().contains("everywhere"));
        assert_eq!(run(&["schema".into(), "nope".into()], &mut Vec::new()), 2);
    }

    #[test]
    fn unknown_settings_are_named_with_a_suggestion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("chat")).unwrap();
        std::fs::write(dir.path().join("chat/providers.json"), r#"{"defaultModle": "x/y", "providers": {"xai": {"enabled": true, "baseURL": "u"}}, "colour": 1}"#).unwrap();
        std::fs::write(dir.path().join("chat/hooks.json"), r#"[{"event": "turn_end", "command": "echo", "enabld": true}]"#).unwrap();
        let w = warnings(dir.path());
        assert!(w.contains(&"chat/providers.json: unknown setting `defaultModle` (did you mean `defaultModel`?)".to_string()), "{w:?}");
        assert!(w.iter().any(|x| x.contains("providers.xai.baseURL") && x.contains("baseUrl")), "{w:?}");
        assert!(w.iter().any(|x| x.ends_with("unknown setting `colour`")), "{w:?}");
        assert!(w.iter().any(|x| x.contains("[0].enabld") && x.contains("`enabled`")), "{w:?}");
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
