//! Cortex's files on disk, with no window, keychain or network: JSON settings files written
//! through a temporary file, encryption at rest (XChaCha20-Poly1305 under a key the app keeps in
//! the OS keychain and hands in), and the errors of both.
//!
//! The app supplies what needs the rest of the system: it reads the key from the keychain and
//! calls [`at_rest::unlock`], and it re-exports these modules where its code expects them
//! (`chat::at_rest`, `support::settings_file`, `support::error`).

pub mod at_rest;
pub mod backup;
pub mod error;
pub mod index;
pub mod partials;
pub mod settings_file;
pub mod store;
pub mod text;
pub mod versions;
