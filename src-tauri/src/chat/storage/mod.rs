//! Where chat data lives: conversations and their index, backups, versions, sync, encryption at rest, the vault and the keychain, uploads and outputs, and imports.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod at_rest;
pub mod audit;
pub mod coding_import;
pub mod import;
pub mod recent_files;
pub mod secrets;
pub mod sync;
pub mod traces;
pub mod uploads;
pub mod vault;

// The conversation store, its index, backups and trash, version history and the partial-answer
// checkpoints live in the cortex-store crate; they are re-exported where the app's code expects them.
pub(super) use cortex_store::{backup, index};
pub use cortex_store::{partials, store, versions};
