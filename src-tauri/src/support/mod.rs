//! Small services every part of the app uses, written once: settings files, and (as later
//! changes move them here) HTTP clients, secrets, names and text helpers.
//!
//! This is the bottom layer. Code here must not import from `chat`, `tools` or the window code,
//! so that any of them can use it. Settings files and their errors live in the `cortex-store`
//! crate, re-exported here.

pub mod dialogs;
pub use cortex_store::{error, settings_file};
pub mod http;
pub mod names;
pub mod text;
