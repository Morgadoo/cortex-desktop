//! The IPC surface agrees with itself.
//!
//! A command the page can call is named in three places by hand: the `generate_handler!` list in
//! `src/app.rs`, a permission in `permissions/*.toml`, and a capability in `capabilities/*.json`
//! that grants that permission to a window. A command missing from either of the last two is
//! refused at run time, not at build time, so this test is the build-time check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files(dir: &str, extension: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("read {dir}: {e}"))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    paths.sort();
    paths
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The command names in `generate_handler![…]`: the last segment of each path.
fn handlers() -> BTreeSet<String> {
    let app = read(&root().join("src/app.rs"));
    let start = app
        .find("generate_handler![")
        .expect("app.rs has a generate_handler! list")
        + "generate_handler![".len();
    let end = start + app[start..].find("];").expect("the handler list ends");
    app[start..end]
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| path.rsplit("::").next().unwrap_or(path).to_string())
        .collect()
}

/// Each permission identifier and the commands it allows, from `permissions/*.toml`.
fn permissions() -> BTreeMap<String, Vec<String>> {
    let block = Regex::new(r#"(?s)\[\[permission\]\](.*?)(?:\n\[\[permission\]\]|\z)"#).unwrap();
    let identifier = Regex::new(r#"(?m)^identifier\s*=\s*"([^"]+)""#).unwrap();
    let allow = Regex::new(r#"(?s)commands\.allow\s*=\s*\[(.*?)\]"#).unwrap();
    let name = Regex::new(r#""([^"]+)""#).unwrap();
    let mut out = BTreeMap::new();
    for path in files("permissions", "toml") {
        let text = read(&path);
        // Blocks are matched one at a time, since each match consumes the next block's header.
        let mut rest = text.as_str();
        while let Some(found) = block.captures(rest) {
            let body = found.get(1).unwrap();
            let id = identifier
                .captures(body.as_str())
                .unwrap_or_else(|| panic!("a permission in {} has no identifier", path.display()))
                [1]
            .to_string();
            let commands = allow
                .captures(body.as_str())
                .map(|c| {
                    name.captures_iter(&c[1])
                        .map(|n| n[1].to_string())
                        .collect()
                })
                .unwrap_or_default();
            assert!(
                out.insert(id.clone(), commands).is_none(),
                "permission {id} is defined twice"
            );
            rest = &rest[body.end()..];
        }
    }
    out
}

/// Each app permission a capability grants, with the capabilities granting it. Plugin
/// permissions (`core:default`, `autostart:allow-enable`) are the plugins' own business.
fn grants() -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in files("capabilities", "json") {
        let capability: Value = serde_json::from_str(&read(&path))
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let name = capability["identifier"].as_str().unwrap_or("?").to_string();
        for entry in capability["permissions"].as_array().into_iter().flatten() {
            let id = entry
                .as_str()
                .or_else(|| entry["identifier"].as_str())
                .unwrap_or_else(|| panic!("a permission in {} has no identifier", path.display()));
            if !id.contains(':') {
                out.entry(id.to_string()).or_default().push(name.clone());
            }
        }
    }
    out
}

#[test]
fn every_handler_has_a_permission() {
    let allowed: BTreeSet<String> = permissions().into_values().flatten().collect();
    let missing: Vec<_> = handlers()
        .into_iter()
        .filter(|h| !allowed.contains(h))
        .collect();
    assert!(
        missing.is_empty(),
        "commands in app.rs that no permission allows (add them to permissions/*.toml): {missing:?}"
    );
}

#[test]
fn every_permitted_command_is_a_handler() {
    let handlers = handlers();
    let stale: Vec<_> = permissions()
        .into_iter()
        .flat_map(|(id, commands)| commands.into_iter().map(move |c| (id.clone(), c)))
        .filter(|(_, command)| !handlers.contains(command))
        .collect();
    assert!(
        stale.is_empty(),
        "permissions naming a command app.rs does not register: {stale:?}"
    );
}

#[test]
fn every_permission_is_granted_and_every_grant_exists() {
    let permissions = permissions();
    let grants = grants();
    let ungranted: Vec<_> = permissions
        .keys()
        .filter(|id| !grants.contains_key(*id))
        .collect();
    assert!(
        ungranted.is_empty(),
        "permissions no capability grants (add them to capabilities/*.json): {ungranted:?}"
    );
    let unknown: Vec<_> = grants
        .keys()
        .filter(|id| !permissions.contains_key(*id))
        .collect();
    assert!(
        unknown.is_empty(),
        "capabilities granting a permission that does not exist: {unknown:?}"
    );
}

#[test]
fn the_parsers_see_the_whole_surface() {
    // A parser that silently matched nothing would make the tests above pass vacuously.
    assert!(
        handlers().len() > 300,
        "found {} handlers",
        handlers().len()
    );
    assert!(
        permissions().len() > 150,
        "found {} permissions",
        permissions().len()
    );
    assert!(grants().len() > 150, "found {} grants", grants().len());
}
