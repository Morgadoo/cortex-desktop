mod account;
mod app;
mod browser_login;
mod chat;
mod dialog;
mod downloads;
mod feature_flags;
mod main_window;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod microphone;
mod notifications;
mod pdf_export;
mod quick_ask;
mod reporting;
mod reset;
mod schedules;
mod settings;
mod skills;
#[cfg(target_os = "macos")]
mod subframe_navigation;
mod theme;
mod tools;
mod tray;
#[cfg(target_os = "macos")]
mod unhandled_keys;
mod updater;
pub mod inapp_browser;
mod webview;

#[doc(hidden)]
pub use tools::connectors::mcp::{
    CustomHeader, HttpAuth, McpConnector, McpServerConfig, McpTransport, NoToolCache,
};

#[doc(hidden)]
pub use tools::{
    auth::{keychain_secrets, CredentialState, Readiness, SecretStore, Secrets},
    error::ToolError,
    registry::{Connector, Session},
};

#[doc(hidden)]
pub use account::AccountId;

pub use app::run;

/// Web app the main webview loads. Overridden at build time via `LUMO_URL`.
pub const LUMO_URL: &str = match option_env!("LUMO_URL") {
    Some(url) => url,
    None => "https://lumo.proton.dev",
};

/// Whether the main window loads the hosted Lumo web app instead of the native chat page. Set
/// `LUMO_HOSTED=1` at build time to get the old behaviour.
pub const HOSTED_WEB_APP: bool = option_env!("LUMO_HOSTED").is_some();

pub fn e2e_testing_enabled() -> bool {
    std::env::var_os("LUMO_E2E_TESTING").is_some_and(|v| !v.is_empty())
}
