//! Small services every part of the app uses, written once: settings files, and (as later
//! changes move them here) HTTP clients, secrets, names and text helpers.
//!
//! This is the bottom layer. Code here must not import from `chat`, `tools` or the window code,
//! so that any of them can use it. One exception remains: [`settings_file`] seals files through
//! `chat::at_rest`, until encryption at rest moves here with its keychain access injected.

pub mod http;
pub mod names;
pub mod settings_file;
