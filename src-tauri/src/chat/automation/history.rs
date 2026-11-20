//! What can be brought back: for each kind of item kept in versions, how to read it as it is now
//! and how to save an earlier version of it again.
//!
//! The versions themselves are kept by [`crate::chat::versions`], which knows nothing of what it
//! stores; this module, above every kind, is the one place that does.

use std::path::Path;

use serde_json::Value;

/// What changed from version `version` of an item to the one after it, or to the item as it is
/// now when `version` is the newest: `current` when the caller has it, else read here.
pub fn diff(
    config_dir: &Path,
    kind: &str,
    id: &str,
    version: &str,
    current: Option<&Value>,
) -> Result<String, String> {
    crate::chat::versions::diff(config_dir, kind, id, version, || {
        current.cloned().or_else(|| now(config_dir, kind, id))
    })
    .map_err(String::from)
}

/// The item as it is now, for diffing its newest version against.
pub(in crate::chat::automation) fn now(config_dir: &Path, kind: &str, id: &str) -> Option<Value> {
    match (kind, id) {
        ("memories", "all") => serde_json::to_value(crate::chat::memory::load(config_dir)).ok(),
        ("knowledge", "pages") => {
            serde_json::to_value(crate::chat::knowledge::load(config_dir)).ok()
        }
        ("instructions", "user") => {
            serde_json::to_value(crate::chat::store::load_instructions(config_dir)).ok()
        }
        ("notebooks", agent) => Some(Value::String(crate::chat::agents::notebook(
            config_dir, agent,
        ))),
        ("agents", agent) => {
            serde_json::to_value(crate::chat::agents::find(config_dir, agent)?).ok()
        }
        ("workflows", workflow) => serde_json::to_value(
            crate::chat::workflows::load(config_dir)
                .into_iter()
                .find(|w| w.id == workflow)?,
        )
        .ok(),
        ("tasks", task) => serde_json::to_value(
            crate::chat::tasks::load(config_dir)
                .into_iter()
                .find(|t| t.id == task)?,
        )
        .ok(),
        ("tools", name) => match name.strip_prefix("script-") {
            Some(script) => {
                serde_json::to_value(crate::chat::script_tools::load(config_dir, script)?).ok()
            }
            None => serde_json::to_value(
                crate::chat::custom_tools::list(config_dir)
                    .into_iter()
                    .find(|t| t.name == name)?,
            )
            .ok(),
        },
        _ => None,
    }
}

/// Brings back version `version` of an item: it is saved again the way the item is always saved,
/// so the version it replaces is kept in turn and the change can itself be undone. The kernel's
/// `revert`, for every kind kept here.
pub fn restore(config_dir: &Path, kind: &str, id: &str, version: &str) -> Result<(), String> {
    let value = crate::chat::versions::list(config_dir, kind, id)?
        .into_iter()
        .find(|v| v.id == version)
        .ok_or("There is no such version.")?
        .value;
    fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
        serde_json::from_value(value)
            .map_err(|e| format!("That version can no longer be read: {e}"))
    }
    match (kind, id) {
        ("memories", "all") => {
            crate::chat::memory::save(config_dir, &parse(value)?).map_err(String::from)
        }
        ("knowledge", "pages") => crate::chat::knowledge::save_all(
            config_dir,
            &parse::<Vec<crate::chat::knowledge::Page>>(value)?,
        )
        .map_err(String::from),
        ("instructions", "user") => {
            crate::chat::store::save_instructions(config_dir, &parse(value)?).map_err(String::from)
        }
        ("notebooks", agent) => {
            crate::chat::agents::save_notebook(config_dir, agent, &parse::<String>(value)?)
        }
        ("agents", _) => crate::chat::agents::save(config_dir, parse(value)?).map(drop),
        ("workflows", _) => crate::chat::workflows::save(config_dir, parse(value)?).map(drop),
        ("tools", name) if name.starts_with("script-") => {
            // The version keeps the hash the user approved for its own script, so an approved
            // version comes back approved and an unapproved one stays a candidate.
            crate::chat::script_tools::restore_version(config_dir, parse(value)?)
                .map_err(String::from)
        }
        ("tools", _) => crate::chat::custom_tools::save(config_dir, &parse(value)?)
            .map(drop)
            .map_err(String::from),
        ("tasks", _) => crate::chat::tasks::save(config_dir, parse(value)?).map(drop),
        _ => Err(format!("{kind} cannot be restored.")),
    }
}
