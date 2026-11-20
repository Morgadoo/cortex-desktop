//! The tool contract, with no window, keychain or network of its own: the [`registry`] and the
//! connector trait every tool source implements, the approval and [`exposure`] rules that decide
//! whether a call may run, [`credentials`] and connector [`state`], tool results kept for reading
//! back, and their [`error`]s.
//!
//! [`echo`] is the smallest complete connector, a reference for writing one.
//!
//! The app supplies the rest: the concrete connectors (MCP servers, the filesystem, bash, the
//! browser, mail), the OS keychain behind [`credentials::SecretStore`], the approval prompt, and
//! saving the state per account. It re-exports these modules under `tools::`.

pub mod child_process;
pub mod command_path;
pub mod credentials;
pub mod echo;
pub mod error;
pub mod exposure;
pub mod hidden_text;
pub mod oauth;
pub mod registry;
pub mod result_reader;
pub mod result_store;
pub mod state;
