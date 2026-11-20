//! HTTP clients: one place that decides how the app identifies itself and how long it waits.
//!
//! Callers choose only the timeout, which depends on the work (a two-second probe of a local
//! server, a ten-minute model download), and whether the request reads a web page.

use std::time::Duration;

/// How the app names itself to the services it calls.
pub const USER_AGENT: &str = concat!("Cortex/", env!("CARGO_PKG_VERSION"));

/// Sent when reading web pages, which often refuse or simplify pages for unknown clients.
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Cortex";

/// A client for an API, giving up on a request after `timeout`.
pub fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    build(
        reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(USER_AGENT),
    )
}

/// A client for reading web pages, giving up on a request after `timeout`.
pub fn browser_client(timeout: Duration) -> Result<reqwest::Client, String> {
    build(
        reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(BROWSER_USER_AGENT),
    )
}

fn build(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, String> {
    builder
        .build()
        .map_err(|e| format!("could not start an HTTP client: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_names_its_version() {
        assert!(super::USER_AGENT.starts_with("Cortex/0."));
        assert!(super::client(std::time::Duration::from_secs(1)).is_ok());
        assert!(super::browser_client(std::time::Duration::from_secs(1)).is_ok());
    }
}
