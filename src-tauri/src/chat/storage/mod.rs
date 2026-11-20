//! Where chat data lives: conversations and their index, backups, versions, sync, encryption at rest, the vault and the keychain, uploads and outputs, and imports.
//!
//! Each module is re-exported from `chat`, so it is also reachable as `crate::chat::<module>`.

pub mod at_rest;
pub mod audit;
pub(super) mod backup;
pub mod coding_import;
pub mod import;
pub(super) mod index;
pub mod partials;
pub mod recent_files;
pub mod secrets;
pub mod store;
pub mod sync;
pub mod traces;
pub mod uploads;
pub mod vault;
pub mod versions;
