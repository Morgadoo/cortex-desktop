//! Tools loaded on demand, as Vibe does ("Found tools", "Loaded tool details").
//!
//! A connector can bring dozens of tools (one here brings over a hundred), and every definition
//! sent costs context on every request. Past [`DEFER_OVER`] connector tools, only the core ones
//! go out with each request; the rest are named by connector in the prompt, and the model loads
//! what it needs with `find_tools`. What it loads stays with the conversation.

use serde_json::{json, Value};

use super::tools::ChatTool;

pub const FIND_TOOLS: &str = "find_tools";
/// Connector tools offered before the rest are loaded on demand.
pub const DEFER_OVER: usize = 30;
/// Tools one search loads.
const MATCHES: usize = 8;
/// Connectors always offered in full: the core of working with files, commands and the web.
const CORE: &[&str] = &[
    "filesystem",
    "bash",
    "browser",
    "tool_results",
    "terminal",
    "sandbox",
];

/// Whether a tool may wait to be loaded: a connector's, outside the core.
pub fn deferrable(tool: &ChatTool) -> bool {
    tool.connector_id
        .as_deref()
        .is_some_and(|c| !CORE.contains(&c))
}

/// Whether this request should load some tools on demand.
pub fn should_defer(offered: &[ChatTool]) -> bool {
    offered.iter().filter(|t| deferrable(t)).count() > DEFER_OVER
}

pub fn find_tools_tool() -> ChatTool {
    ChatTool {
        name: FIND_TOOLS.into(),
        description: "Find and load tools that are not loaded yet (see \"More tools\" in your instructions). \
            Describe what you need to do; the best matches are loaded and can be called from your next step."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": { "query": { "type": "string", "description": "What the tool should do, e.g. \"create a calendar event\"." } },
            "required": ["query"]
        }),
        connector_id: None,
        tool_name: FIND_TOOLS.into(),
        read_only: true,
    }
}

/// The deferred tools that best match `query`, by shared words in name and description.
pub fn search<'a>(offered: &'a [ChatTool], query: &str, limit: usize) -> Vec<&'a ChatTool> {
    use super::retrieval::{tokens, Corpus};
    let candidates: Vec<&ChatTool> = offered.iter().filter(|t| deferrable(t)).collect();
    // The name counts three times; words match as prefixes, so `email` finds `emails`.
    let corpus = Corpus::new(candidates.iter().map(|t| {
        let name = tokens(&t.name.replace("__", " ").replace('_', " "));
        let mut words = name.clone();
        words.extend(name.clone());
        words.extend(name);
        words.extend(tokens(&t.description));
        words
    }))
    .with_prefixes();
    corpus
        .search(query)
        .into_iter()
        .take(limit)
        .map(|(i, _)| candidates[i])
        .collect()
}

/// Runs `find_tools`: the matches, as the tool result, and the names to load.
#[cfg(test)]
pub fn find(offered: &[ChatTool], args: &Value) -> (String, Vec<String>) {
    find_with(offered, args, None)
}

/// [`find`], with a semantic ranking of the deferrable tools (best first, by name) fused in.
pub fn find_with(
    offered: &[ChatTool],
    args: &Value,
    semantic: Option<Vec<String>>,
) -> (String, Vec<String>) {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let lexical = search(offered, query, MATCHES * 3);
    let found: Vec<&ChatTool> = match semantic {
        Some(semantic) => {
            let candidates: Vec<&ChatTool> = offered.iter().filter(|t| deferrable(t)).collect();
            let index = |name: &str| candidates.iter().position(|t| t.name == name);
            let by_words: Vec<usize> = lexical.iter().filter_map(|t| index(&t.name)).collect();
            let by_meaning: Vec<usize> = semantic
                .iter()
                .filter_map(|n| index(n))
                .take(MATCHES * 3)
                .collect();
            super::retrieval::fuse_ranks(&[by_words, by_meaning])
                .into_iter()
                .take(MATCHES)
                .map(|i| candidates[i])
                .collect()
        }
        None => lexical.into_iter().take(MATCHES).collect(),
    };
    if found.is_empty() {
        return (
            json!({ "query": query, "matches": [], "note": "No tool matched; try other words." })
                .to_string(),
            Vec::new(),
        );
    }
    let matches: Vec<Value> = found
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description.chars().take(300).collect::<String>() }))
        .collect();
    let text = json!({ "query": query, "matches": matches, "note": "These tools are loaded; call them from your next step." }).to_string();
    (text, found.iter().map(|t| t.name.clone()).collect())
}

/// The part of the instructions that says which tools wait to be loaded.
pub fn prompt(offered: &[ChatTool], loaded: &[String]) -> String {
    let mut by_connector: Vec<(String, usize)> = Vec::new();
    for tool in offered
        .iter()
        .filter(|t| deferrable(t) && !loaded.contains(&t.name))
    {
        let connector = tool.connector_id.clone().unwrap_or_default();
        match by_connector.iter_mut().find(|(c, _)| *c == connector) {
            Some((_, n)) => *n += 1,
            None => by_connector.push((connector, 1)),
        }
    }
    if by_connector.is_empty() {
        return String::new();
    }
    let list: Vec<String> = by_connector
        .iter()
        .map(|(c, n)| format!("{} ({n})", c.trim_start_matches("mcp-")))
        .collect();
    format!(
        "\n\n## More tools\nMore tools are available but not loaded, to keep requests small: {}. When the task needs \
         one of these services, call `{FIND_TOOLS}` with what you need to do; the matches load for your next step.",
        list.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(connector: &str, name: &str, description: &str) -> ChatTool {
        ChatTool {
            name: format!("{connector}__{name}"),
            description: description.into(),
            parameters: json!({}),
            connector_id: Some(connector.into()),
            tool_name: name.into(),
            read_only: false,
        }
    }

    #[test]
    fn defers_only_past_the_limit_and_finds_by_words() {
        let mut offered = vec![tool("filesystem", "fs_read", "Read a file")];
        for i in 0..DEFER_OVER {
            offered.push(tool("mcp-crm", &format!("op_{i}"), "Generic operation"));
        }
        assert!(
            !should_defer(&offered),
            "at the limit everything is offered"
        );
        offered.push(tool(
            "mcp-calendar",
            "create_event",
            "Create a calendar event with attendees",
        ));
        assert!(should_defer(&offered));
        assert!(
            !deferrable(&offered[0]),
            "core connectors are never deferred"
        );

        let (text, loaded) = find(&offered, &json!({ "query": "schedule a calendar event" }));
        assert_eq!(loaded, vec!["mcp-calendar__create_event".to_string()]);
        assert!(text.contains("create_event"));
        let (_, none) = find(&offered, &json!({ "query": "zzz" }));
        assert!(none.is_empty());

        let p = prompt(&offered, &loaded);
        assert!(p.contains("crm (30)") && !p.contains("calendar"));
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    /// A semantic ranking finds a tool its words alone would miss, and both rankings are fused.
    #[test]
    fn meaning_and_words_are_fused() {
        let tool = |name: &str, description: &str| ChatTool {
            name: name.into(),
            description: description.into(),
            parameters: json!({}),
            connector_id: Some("mcp-x".into()),
            tool_name: name.into(),
            read_only: true,
        };
        let offered = vec![
            tool("mcp-x__send_mail", "Send an email message."),
            tool("mcp-x__list_events", "List calendar events."),
        ];
        let args = json!({ "query": "my schedule tomorrow" });
        // Words alone find nothing for "schedule".
        assert!(find(&offered, &args).1.is_empty());
        let semantic = Some(vec![
            "mcp-x__list_events".to_string(),
            "mcp-x__send_mail".to_string(),
        ]);
        let (_, names) = find_with(&offered, &args, semantic);
        assert_eq!(names[0], "mcp-x__list_events");
    }
}
