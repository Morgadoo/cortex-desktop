//! The HTTP MCP servers the app offers in its catalog.
//!
//! A catalog server is an established service the app vouches for, so its read-only hints are
//! believed. A server the user added by hand can label any tool read-only, and can put
//! instructions for the model in its descriptions.

use super::config::{McpServerConfig, McpTransport};
use serde::Deserialize;
use std::sync::LazyLock;

/// The shared catalog's entries; only their URLs matter here.
#[derive(Deserialize)]
struct Catalog {
    builtin: Vec<Entry>,
    featured: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    url: Option<String>,
}

/// Read from the catalog the settings page and the web app are built from, so the two can't
/// drift apart.
static CATALOG_URLS: LazyLock<Vec<String>> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(include_str!(
        "../../../../../webview/shared/connector-catalog.json"
    ))
    .expect("webview/shared/connector-catalog.json is valid");
    catalog
        .builtin
        .into_iter()
        .chain(catalog.featured)
        .filter_map(|entry| entry.url)
        .map(|url| normalize(&url))
        .collect()
});

fn normalize(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `config` is a catalog entry: an HTTP server at exactly a catalog URL. Stdio servers
/// and URLs the user typed in count as added by hand.
pub(crate) fn is_catalog_server(config: &McpServerConfig) -> bool {
    match &config.transport {
        McpTransport::Http { url, .. } => CATALOG_URLS.contains(&normalize(url)),
        McpTransport::Stdio { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_read_from_the_shared_list() {
        assert!(CATALOG_URLS.len() >= 20, "{:?}", *CATALOG_URLS);
        assert!(CATALOG_URLS.contains(&"https://mcp.slack.com/mcp".to_string()));
        assert!(CATALOG_URLS.iter().all(|url| url.starts_with("http")));
    }
}
