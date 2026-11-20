//! Tools the app itself provides, beside the connectors: web search, memory and skills.

use std::path::Path;

use serde_json::{json, Value};

use crate::chat::tools::{ChatTool, Outcome};
use crate::chat::{memory, search, skills};

pub use crate::chat::tool_names::*;

/// What a built-in can change, which decides how it meets the approval mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, searches or shows something, and changes nothing.
    Reads,
    /// Writes the assistant's own memory, which the user can switch off and edit in Settings.
    /// It never asks, so the assistant can keep notes as it goes.
    Memory,
    /// Leaves something behind that acts later or changes how the assistant works: a scheduled
    /// task, a saved skill. It follows the approval mode like a connector tool that changes
    /// something.
    Standing,
}

pub fn effect(name: &str) -> Effect {
    match name {
        REMEMBER | FORGET | KNOWLEDGE_WRITE => Effect::Memory,
        SCHEDULE_TASK | SAVE_SKILL => Effect::Standing,
        _ => Effect::Reads,
    }
}

fn ok(text: impl Into<String>) -> Outcome {
    Outcome::ok(text)
}

fn err(text: impl Into<String>) -> Outcome {
    Outcome::err(text)
}

/// A string argument, trimmed; empty when it is missing.
fn text_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Runs the built-in `name`: one function per tool below.
pub async fn run(config_dir: &Path, name: &str, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    if matches!(name, REMEMBER | FORGET | KNOWLEDGE_WRITE) && !memory::load(config_dir).update {
        return err("The user has turned off saving to knowledge; nothing was changed.");
    }
    match name {
        WEB_SEARCH => web_search(config_dir, args).await,
        REMEMBER => remember(config_dir, args).await,
        FORGET => forget(config_dir, args).await,
        SCHEDULE_TASK => schedule(config_dir, args),
        KNOWLEDGE_WRITE => knowledge_write(config_dir, args).await,
        KNOWLEDGE_READ => knowledge_read(config_dir, args).await,
        KNOWLEDGE_SEARCH => knowledge_search(config_dir, args).await,
        READ_DOCUMENT => read_document(config_dir, args).await,
        SEARCH_CHATS => search_chats(config_dir, args).await,
        MEMORY_SEARCH => memory_search(config_dir, args).await,
        QUERY_TRACES => query_traces(config_dir, args).await,
        GET_RUN => get_run(config_dir, args).await,
        USAGE_STATS if !str_arg("artifact").is_empty() => {
            usage_stats_of_artifact(config_dir, args).await
        }
        USAGE_STATS | FAILURE_SUMMARY => usage_or_failures(config_dir, args, name).await,
        READ_CHAT => read_chat(config_dir, args).await,
        SAVE_SKILL => save_skill(config_dir, args).await,
        CALENDAR_EVENTS => calendar_events(config_dir, args).await,
        FIND_CONTACT => find_contact(config_dir, args).await,
        SEARCH_LIBRARY => search_library(config_dir, args).await,
        UPDATE_PLAN => update_plan(config_dir, args).await,
        USE_SKILL => use_skill(config_dir, args).await,
        _ => err(format!("Unknown built-in tool {name}.")),
    }
}

/// `WEB_SEARCH`.
async fn web_search(config_dir: &Path, args: &Value) -> Outcome {
    let str_arg = |key: &str| text_arg(args, key);
    let query = str_arg("query");
    if query.is_empty() {
        return err("Give a search query.");
    }
    let max = args
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(6)
        .clamp(1, 10) as usize;
    let recency = search::Recency::parse(&str_arg("recency"));
    let sites: Vec<String> = args
        .get("sites")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let sites = search::clean_sites(&sites);
    match search::search_sites(config_dir, &query, max, recency, &sites).await {
        Ok(hits) if hits.is_empty() && !sites.is_empty() => ok(format!(
            "No results on {} for that query. Try other words or search without sites.",
            sites.join(", ")
        )),
        Ok(hits) => ok(serde_json::to_string_pretty(
            &json!({ "query": query, "sites": sites, "results": hits }),
        )
        .unwrap_or_default()),
        Err(e) => err(e),
    }
}

/// How much of an earlier conversation `read_chat` returns.
const READ_CHAT_LIMIT: usize = 24_000;

mod insight;
mod recall;
mod schemas;
mod work;
use insight::*;
use recall::*;
pub use schemas::*;
pub(crate) use work::*;

#[cfg(test)]
mod tests;
